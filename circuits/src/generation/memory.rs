use mozak_vm::instruction::Op;
use mozak_vm::vm::Row;
use plonky2::hash::hash_types::RichField;

use crate::memory::columns as mem_cols;
use crate::memory::trace::{
    get_memory_inst_addr, get_memory_inst_clk, get_memory_inst_op, get_memory_load_inst_value,
    get_memory_store_inst_value,
};
use crate::utils::pad_trace;

/// Returns the rows sorted in the order of the instruction address.
#[must_use]
pub fn filter_memory_trace(mut step_rows: Vec<Row>) -> Vec<Row> {
    // Sorting is stable, and rows are already ordered by row.state.clk
    step_rows.sort_by_key(|row| {
        row.state
            .get_register_value(row.inst.data.rs1.into())
            .wrapping_add(row.inst.data.imm)
    });

    step_rows
        .into_iter()
        .filter(|row| row.inst.op == Op::LB || row.inst.op == Op::SB)
        .collect()
}

#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn generate_memory_trace<F: RichField>(
    step_rows: Vec<Row>,
) -> [Vec<F>; mem_cols::NUM_MEM_COLS] {
    let filtered_step_rows = filter_memory_trace(step_rows);

    let mut trace: Vec<Vec<F>> =
        vec![vec![F::ZERO; filtered_step_rows.len()]; mem_cols::NUM_MEM_COLS];
    for (i, s) in filtered_step_rows.iter().enumerate() {
        trace[mem_cols::COL_MEM_PADDING][i] = F::ZERO;
        trace[mem_cols::COL_MEM_ADDR][i] = get_memory_inst_addr(s);
        trace[mem_cols::COL_MEM_CLK][i] = get_memory_inst_clk(s);
        trace[mem_cols::COL_MEM_OP][i] = get_memory_inst_op(&s.inst);

        trace[mem_cols::COL_MEM_VALUE][i] = match s.inst.op {
            Op::LB => get_memory_load_inst_value(s),
            Op::SB => get_memory_store_inst_value(s),
            _ => F::ZERO,
        };

        trace[mem_cols::COL_MEM_DIFF_ADDR][i] = if i == 0 {
            F::ZERO
        } else {
            trace[mem_cols::COL_MEM_ADDR][i] - trace[mem_cols::COL_MEM_ADDR][i - 1]
        };

        trace[mem_cols::COL_MEM_DIFF_CLK][i] =
            if i == 0 || trace[mem_cols::COL_MEM_ADDR][i] != trace[mem_cols::COL_MEM_ADDR][i - 1] {
                F::ZERO
            } else {
                trace[mem_cols::COL_MEM_CLK][i] - trace[mem_cols::COL_MEM_CLK][i - 1]
            };
    }

    // If the trace length is not a power of two, we need to extend the trace to the
    // next power of two. The additional elements are filled with the last row
    // of the trace.
    let trace = pad_trace(trace, Some(mem_cols::COL_MEM_PADDING));

    trace.try_into().unwrap_or_else(|v: Vec<Vec<F>>| {
        panic!(
            "Expected a Vec of length {} but it was {}",
            mem_cols::NUM_MEM_COLS,
            v.len()
        )
    })
}

#[cfg(test)]
mod test {
    use mozak_vm::test_utils::simple_test;
    use plonky2::field::types::Field;
    use plonky2::plonk::config::{GenericConfig, PoseidonGoldilocksConfig};

    // This test simulates the scenario of a set of instructions
    // which perform store byte (SB) and load byte (LB) operations
    // to memory and then checks if the memory trace is generated correctly.
    #[test]
    fn generate_memory_trace() {
        // PADDING  ADDR       CLK       OP        VALUE     DIFF_ADDR   DIFF_CLK
        // 0        100        0         SB        5         0           0
        // 0        100        4         LB        5         0           4
        // 0        100        16        SB        10        0           12
        // 0        100        20        LB        10        0           4
        // 0        200        8         SB        15        100         0
        // 0        200        12        LB        15        0           4
        let (rows, state) = simple_test(
            24,
            &[
                // Store Byte: M[rs1 + imm] = rs2
                // imm[11:5]  rs2    rs1    funct3  imm[4:0]  opcode
                // Load Byte: rd = M[rs1 + imm]
                // imm[11:0]         rs1    funct3  rd        opcode
                // 0000011    00001  00000  000     00100     0100011   sb r1, 100(r0)
                // 000001100100      00000  000     00100     0000011   lb r4, 100(r0)
                // 0000011    00010  00000  000     00100     0100011   sb r2, 100(r0)
                // 000001100100      00000  000     00101     0000011   lb r5, 100(r0)
                // 0000110    00011  00000  000     01000     0100011   sb r3, 200(r0)
                // 000011001000      00000  000     00110     0000011   lb r6, 200(r0)
                (0_u32, 0b00000110000100000000001000100011),
                (4_u32, 0b00000110010000000000001000000011),
                (16_u32, 0b00000110001000000000001000100011),
                (20_u32, 0b00000110010000000000001010000011),
                (8_u32, 0b00001100001100000000010000100011),
                (12_u32, 0b00001100100000000000001100000011),
            ],
            &[(1, 5), (2, 10), (3, 15)],
        );
        assert_eq!(state.load_u8(100), 10);
        assert_eq!(state.get_register_value(4), 5);
        assert_eq!(state.get_register_value(5), 10);
        assert_eq!(state.load_u8(200), 15);
        assert_eq!(state.get_register_value(6), 15);

        const D: usize = 2;
        type C = PoseidonGoldilocksConfig;
        type F = <C as GenericConfig<D>>::F;
        let trace = super::generate_memory_trace::<F>(rows);
        let expected_trace = [
            [
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::ONE,
                F::ONE,
            ],
            [
                F::from_canonical_u32(100),
                F::from_canonical_u32(100),
                F::from_canonical_u32(100),
                F::from_canonical_u32(100),
                F::from_canonical_u32(200),
                F::from_canonical_u32(200),
                F::from_canonical_u32(200),
                F::from_canonical_u32(200),
            ],
            [
                F::from_canonical_u32(1),
                F::from_canonical_u32(2),
                F::from_canonical_u32(5),
                F::from_canonical_u32(6),
                F::from_canonical_u32(3),
                F::from_canonical_u32(4),
                F::from_canonical_u32(4),
                F::from_canonical_u32(4),
            ],
            [
                F::ONE,
                F::ZERO,
                F::ONE,
                F::ZERO,
                F::ONE,
                F::ZERO,
                F::ZERO,
                F::ZERO,
            ],
            [
                F::from_canonical_u32(5),
                F::from_canonical_u32(5),
                F::from_canonical_u32(10),
                F::from_canonical_u32(10),
                F::from_canonical_u32(15),
                F::from_canonical_u32(15),
                F::from_canonical_u32(15),
                F::from_canonical_u32(15),
            ],
            [
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::ZERO,
                F::from_canonical_u32(100),
                F::ZERO,
                F::ZERO,
                F::ZERO,
            ],
            [
                F::from_canonical_u32(0),
                F::from_canonical_u32(1),
                F::from_canonical_u32(3),
                F::from_canonical_u32(1),
                F::from_canonical_u32(0),
                F::from_canonical_u32(1),
                F::from_canonical_u32(1),
                F::from_canonical_u32(1),
            ],
        ];
        assert_eq!(trace, expected_trace);
    }
}
