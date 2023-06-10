use std::marker::PhantomData;

use anyhow::Result;
use plonky2::field::extension::FieldExtension;
use plonky2::field::packed::PackedField;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::{field::extension::Extendable, hash::hash_types::RichField};
use starky::constraint_consumer::{ConstraintConsumer, RecursiveConstraintConsumer};
use starky::stark::Stark;
use starky::vars::{StarkEvaluationTargets, StarkEvaluationVars};

use super::{columns::*, *};

#[derive(Copy, Clone, Default)]
pub struct CpuStark<F, const D: usize> {
    compress_challenge: Option<F>,
    pub f: PhantomData<F>,
}

impl<F: RichField, const D: usize> CpuStark<F, D> {
    pub fn set_compress_challenge(&mut self, challenge: F) -> Result<()> {
        assert!(self.compress_challenge.is_none(), "already set?");
        self.compress_challenge = Some(challenge);
        Ok(())
    }
    pub fn get_compress_challenge(&self) -> Option<F> {
        self.compress_challenge
    }
}

impl<F: RichField + Extendable<D>, const D: usize> CpuStark<F, D> {
    /// Selector of opcode, builtins and halt should be one-hot encoded.
    ///
    /// Ie exactly one of them should be by 1, all others by 0 in each row.
    /// See <https://en.wikipedia.org/wiki/One-hot>
    fn opcode_one_hot<FE, P, const D2: usize>(lv: &[P], yield_constr: &mut ConstraintConsumer<P>)
    where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>,
    {
        let op_selectors = [lv[COL_S_ADD], lv[COL_S_HALT]];

        op_selectors
            .iter()
            .for_each(|s| yield_constr.constraint(*s * (P::ONES - *s)));

        // Only one opcode selector enabled.
        let sum_s_op: P = op_selectors.into_iter().sum();
        yield_constr.constraint(P::ONES - sum_s_op);
    }

    /// Ensure clock is ticking up
    fn clock_ticks<FE, P, const D2: usize>(
        lv: &[P],
        nv: &[P],
        yield_constr: &mut ConstraintConsumer<P>,
    ) where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>,
    {
        yield_constr.constraint(nv[COL_CLK] - (lv[COL_CLK] + P::ONES));
    }
    /// Register used as destination register can have different value, all
    /// other regs have same value as of previous row.
    fn only_rd_changes<FE, P, const D2: usize>(
        lv: &[P],
        nv: &[P],
        yield_constr: &mut ConstraintConsumer<P>,
    ) where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>,
    {
        for reg in 0..32 {
            let reg_index = COL_REGS.start + reg;
            yield_constr.constraint(
                (lv[COL_RD] - P::Scalar::from_canonical_u32(reg as u32))
                    * (lv[reg_index] - nv[reg_index]),
            );
        }
    }
}

impl<F: RichField + Extendable<D>, const D: usize> Stark<F, D> for CpuStark<F, D> {
    const COLUMNS: usize = NUM_CPU_COLS;
    const PUBLIC_INPUTS: usize = 0;

    fn eval_packed_generic<FE, P, const D2: usize>(
        &self,
        vars: StarkEvaluationVars<FE, P, { Self::COLUMNS }, { Self::PUBLIC_INPUTS }>,
        yield_constr: &mut ConstraintConsumer<P>,
    ) where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>,
    {
        let lv = vars.local_values;
        let nv = vars.next_values;

        Self::opcode_one_hot(lv, yield_constr);

        Self::clock_ticks(lv, nv, yield_constr);

        // Registers
        Self::only_rd_changes(lv, nv, yield_constr);


        // add constraint
        add::eval_packed_generic(lv, nv, yield_constr);
        halt::eval_packed_generic(lv, nv, yield_constr);

        // Last row must be HALT
        yield_constr.constraint_last_row(lv[COL_S_HALT] - P::ONES);
    }

    fn constraint_degree(&self) -> usize {
        2
    }

    fn eval_ext_circuit(
        &self,
        _builder: &mut CircuitBuilder<F, D>,
        _vars: StarkEvaluationTargets<D, { Self::COLUMNS }, { Self::PUBLIC_INPUTS }>,
        _yield_constr: &mut RecursiveConstraintConsumer<F, D>,
    ) {
        unimplemented!()
    }
}
