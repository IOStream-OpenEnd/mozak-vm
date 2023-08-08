use std::borrow::Borrow;
use std::marker::PhantomData;

use itertools::izip;
use plonky2::field::extension::{Extendable, FieldExtension};
use plonky2::field::packed::PackedField;
use plonky2::field::types::Field;
use plonky2::hash::hash_types::RichField;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use starky::constraint_consumer::{ConstraintConsumer, RecursiveConstraintConsumer};
use starky::stark::Stark;
use starky::vars::{StarkEvaluationTargets, StarkEvaluationVars};

use super::columns::{CpuColumnsExtended, CpuColumnsView, InstructionView, OpSelectorView};
use super::{add, bitwise, branches, div, ecall, jalr, mul, signed_comparison, sub};
use crate::columns_view::NumberOfColumns;
use crate::program::columns::ProgramColumnsView;

#[derive(Copy, Clone, Default)]
#[allow(clippy::module_name_repetitions)]
pub struct CpuStark<F, const D: usize> {
    pub _f: PhantomData<F>,
}

impl<P: Copy + core::ops::Add<Output = P>> OpSelectorView<P> {
    // Note: ecall is only 'jumping' in the sense that a 'halt' does not bump the
    // PC. It sort-of jumps back to itself.
    fn is_straightline(&self) -> P {
        self.add
            + self.sub
            + self.and
            + self.or
            + self.xor
            + self.divu
            + self.mul
            + self.mulhu
            + self.remu
            + self.sll
            + self.slt
            + self.sltu
            + self.srl
    }
}

fn pc_ticks_up<P: PackedField>(
    lv: &CpuColumnsView<P>,
    nv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    yield_constr.constraint_transition(
        lv.inst.ops.is_straightline()
            * (nv.inst.pc - (lv.inst.pc + P::Scalar::from_noncanonical_u64(4))),
    );
}

/// Selector of opcode, and registers should be one-hot encoded.
///
/// Ie exactly one of them should be by 1, all others by 0 in each row.
/// See <https://en.wikipedia.org/wiki/One-hot>
fn one_hots<P: PackedField>(inst: &InstructionView<P>, yield_constr: &mut ConstraintConsumer<P>) {
    one_hot(inst.ops, yield_constr);
    one_hot(inst.rs1_select, yield_constr);
    one_hot(inst.rs2_select, yield_constr);
    one_hot(inst.rd_select, yield_constr);
}

fn one_hot<P: PackedField, Selectors: Clone + IntoIterator<Item = P>>(
    selectors: Selectors,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    // selectors have value 0 or 1.
    selectors
        .clone()
        .into_iter()
        .for_each(|s| yield_constr.constraint(s * (P::ONES - s)));

    // Only one selector enabled.
    let sum_s_op: P = selectors.into_iter().sum();
    yield_constr.constraint(P::ONES - sum_s_op);
}

/// Ensure clock is ticking up, iff CPU is still running.
fn clock_ticks<P: PackedField>(
    lv: &CpuColumnsView<P>,
    nv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    let clock_diff = nv.clk - lv.clk;
    let still_running = P::ONES - lv.halt;
    yield_constr.constraint_transition(clock_diff - still_running);
}

/// Register 0 is always 0
fn r0_always_0<P: PackedField>(lv: &CpuColumnsView<P>, yield_constr: &mut ConstraintConsumer<P>) {
    yield_constr.constraint(lv.regs[0]);
}

/// Ensures that if [`filter`] is 0, then duplicate instructions
/// are present. Note that this function doesn't check whether every instruction
/// is unique. Rather, it ensures that no unique instruction present in the
/// trace is omitted. It also doesn't verify the execution order of the
/// instructions.
fn check_permuted_inst_cols<P: PackedField>(
    lv: &ProgramColumnsView<P>,
    nv: &ProgramColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    yield_constr.constraint(lv.filter * (lv.filter - P::ONES));
    yield_constr.constraint_first_row(lv.filter - P::ONES);

    for (lv_col, nv_col) in izip![lv.inst, nv.inst] {
        yield_constr.constraint((nv.filter - P::ONES) * (lv_col - nv_col));
    }
}

/// Register used as destination register can have different value, all
/// other regs have same value as of previous row.
fn only_rd_changes<P: PackedField>(
    lv: &CpuColumnsView<P>,
    nv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    // Note: register 0 is already always 0.
    // But we keep the constraints simple here.
    (0..32).for_each(|reg| {
        yield_constr.constraint_transition(
            (P::ONES - lv.inst.rd_select[reg]) * (lv.regs[reg] - nv.regs[reg]),
        );
    });
}

fn rd_actually_changes<P: PackedField>(
    lv: &CpuColumnsView<P>,
    nv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    // Note: we skip 0 here, because it's already forced to 0 permanently by
    // `r0_always_0`
    (1..32).for_each(|reg| {
        yield_constr
            .constraint_transition((lv.inst.rd_select[reg]) * (lv.dst_value - nv.regs[reg]));
    });
}

fn populate_op1_value<P: PackedField>(
    lv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    yield_constr.constraint(
        lv.op1_value
            // Note: we could skip 0, because r0 is always 0.
            // But we keep the constraints simple here.
            - (0..32)
                .map(|reg| lv.inst.rs1_select[reg] * lv.regs[reg])
                .sum::<P>(),
    );
}

/// `OP2_VALUE` is the sum of the value of the second operand register and the
/// immediate value.
fn populate_op2_value<P: PackedField>(
    lv: &CpuColumnsView<P>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    yield_constr.constraint(
        lv.op2_value - lv.inst.imm_value
            // Note: we could skip 0, because r0 is always 0.
            // But we keep the constraints simple here.
            - (0..32)
                .map(|reg| lv.inst.rs2_select[reg] * lv.regs[reg])
                .sum::<P>(),
    );
}

impl<F: RichField + Extendable<D>, const D: usize> Stark<F, D> for CpuStark<F, D> {
    const COLUMNS: usize = CpuColumnsExtended::<F>::NUMBER_OF_COLUMNS;
    const PUBLIC_INPUTS: usize = 0;

    #[allow(clippy::similar_names)]
    fn eval_packed_generic<FE, P, const D2: usize>(
        &self,
        vars: StarkEvaluationVars<FE, P, { Self::COLUMNS }, { Self::PUBLIC_INPUTS }>,
        yield_constr: &mut ConstraintConsumer<P>,
    ) where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>, {
        let lv: &CpuColumnsExtended<_> = vars.local_values.borrow();
        let nv: &CpuColumnsExtended<_> = vars.next_values.borrow();

        check_permuted_inst_cols(&lv.permuted, &nv.permuted, yield_constr);

        let lv = &lv.cpu;
        let nv = &nv.cpu;

        clock_ticks(lv, nv, yield_constr);
        pc_ticks_up(lv, nv, yield_constr);

        one_hots(&lv.inst, yield_constr);

        // Registers
        r0_always_0(lv, yield_constr);
        only_rd_changes(lv, nv, yield_constr);
        rd_actually_changes(lv, nv, yield_constr);
        populate_op1_value(lv, yield_constr);
        populate_op2_value(lv, yield_constr);

        // add constraint
        add::constraints(lv, yield_constr);
        sub::constraints(lv, yield_constr);
        bitwise::constraints(lv, yield_constr);
        branches::comparison_constraints(lv, yield_constr);
        branches::constraints(lv, nv, yield_constr);
        signed_comparison::signed_constraints(lv, yield_constr);
        signed_comparison::slt_constraints(lv, yield_constr);
        div::constraints(lv, yield_constr);
        mul::constraints(lv, yield_constr);
        jalr::constraints(lv, nv, yield_constr);
        ecall::constraints(lv, nv, yield_constr);

        // Last row must be HALT
        yield_constr.constraint_last_row(lv.halt - P::ONES);
    }

    fn constraint_degree(&self) -> usize { 3 }

    #[no_coverage]
    fn eval_ext_circuit(
        &self,
        _builder: &mut CircuitBuilder<F, D>,
        _vars: StarkEvaluationTargets<D, { Self::COLUMNS }, { Self::PUBLIC_INPUTS }>,
        _yield_constr: &mut RecursiveConstraintConsumer<F, D>,
    ) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use plonky2::plonk::config::{GenericConfig, PoseidonGoldilocksConfig};
    use starky::stark_testing::test_stark_low_degree;

    use crate::cpu::stark::CpuStark;

    #[test]
    fn test_degree() -> Result<()> {
        const D: usize = 2;
        type C = PoseidonGoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;
        type S = CpuStark<F, D>;

        let stark = S::default();
        test_stark_low_degree(stark)
    }
}
