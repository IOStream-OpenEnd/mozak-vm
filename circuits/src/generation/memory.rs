use itertools::{self, Itertools};
use mozak_runner::elf::Program;
use mozak_runner::vm::Row;
use plonky2::hash::hash_types::RichField;

use crate::memory::columns::Memory;
use crate::memory::trace::{get_memory_inst_addr, get_memory_inst_clk, get_memory_inst_op};

/// Pad the memory trace to a power of 2.
#[must_use]
fn pad_mem_trace<F: RichField>(mut trace: Vec<Memory<F>>) -> Vec<Memory<F>> {
    trace.resize(trace.len().next_power_of_two(), Memory {
        // Some columns need special treatment..
        is_executed: F::ZERO,
        is_init: F::ZERO,
        diff_addr: F::ZERO,
        diff_addr_inv: F::ZERO,
        diff_clk: F::ZERO,
        // .. and all other columns just have their last value duplicated.
        ..trace.last().copied().unwrap_or_default()
    });
    trace
}

/// Generates Memory trace from dynamic VM execution of
/// `Program`. These need to be further interleaved with
/// static memory trace generated from `Program` for final
/// execution for final memory trace.
#[must_use]
pub fn generate_memory_trace_from_execution<F: RichField>(
    program: &Program,
    step_rows: &[Row],
) -> Vec<Memory<F>> {
    step_rows
        .iter()
        .filter(|row| row.aux.mem_addr.is_some())
        .map(|row| {
            let addr: F = get_memory_inst_addr(row);
            let addr_u32: Result<u32, _> = addr.to_canonical_u64().try_into();
            Memory {
                is_executed: F::ONE,
                is_writable: F::from_bool(program.rw_memory.contains_key(&addr_u32.unwrap())),
                is_init: F::ZERO,
                addr,
                clk: get_memory_inst_clk(row),
                op: get_memory_inst_op(&(row.state).current_instruction(program)),
                value: F::from_canonical_u32(row.aux.dst_val),
                diff_addr: F::ZERO,     // To be fixed later during interleaving
                diff_addr_inv: F::ZERO, // To be fixed later during interleaving
                diff_clk: F::ZERO,      // To be fixed later during interleaving
            }
        })
        .sorted_by_key(|memory| memory.addr.to_canonical_u64())
        .collect()
}

/// Generates Memory trace from static `Program` for both read-only
/// and read-write memory initializations. These need to be further
/// interleaved with runtime memory trace generated from VM
/// execution for final memory trace.
#[must_use]
pub fn generate_memory_init_trace_from_program<F: RichField>(program: &Program) -> Vec<Memory<F>> {
    [(F::ZERO, &program.ro_memory), (F::ONE, &program.rw_memory)]
        .into_iter()
        .flat_map(|(is_writable, mem)| {
            mem.iter().map(move |(&addr, &value)| Memory {
                is_executed: F::ONE,
                is_writable,
                is_init: F::ONE,
                addr: F::from_canonical_u32(addr),
                clk: F::ZERO,
                op: F::ZERO,
                value: F::from_canonical_u8(value),
                diff_addr: F::ZERO,     // To be fixed later during interleaving
                diff_addr_inv: F::ZERO, // To be fixed later during interleaving
                diff_clk: F::ZERO,      // To be fixed later during interleaving
            })
        })
        .sorted_by_key(|memory| memory.addr.to_canonical_u64())
        .collect()
}

/// Generates memory trace using static component `program` for
/// memory initialization and dynamic component `step_rows` for
/// access (load and store) of memory elements. Trace constraints
/// are supposed to abide by read-only and read-write address
/// constraints.
#[must_use]
pub fn generate_memory_trace<F: RichField>(program: &Program, step_rows: &[Row]) -> Vec<Memory<F>> {
    // `merged_trace` is address sorted combination of static and
    // dynamic memory trace components of program (ELF and execution)
    // `merge` operation is expected to be stable
    let mut merged_trace: Vec<Memory<F>> = generate_memory_init_trace_from_program::<F>(program)
        .into_iter()
        .merge_by(
            generate_memory_trace_from_execution(program, step_rows),
            |x, y| {
                let (x_u64, y_u64) = (x.addr.to_canonical_u64(), y.addr.to_canonical_u64());
                if x_u64 == y_u64 {
                    x.is_init.to_canonical_u64() == 1
                } else {
                    x_u64 < y_u64
                }
            },
        )
        .collect();

    // Ensures constraints by filling remaining inter-row
    // relation values: clock difference and addr difference
    let mut last_clk = F::ZERO;
    let mut last_addr = F::ZERO;
    for mem in &mut merged_trace {
        mem.diff_addr = mem.addr - last_addr;
        mem.diff_addr_inv = mem.diff_addr.try_inverse().unwrap_or_default();
        if mem.addr == last_addr {
            mem.diff_clk = mem.clk - last_clk;
        }
        (last_clk, last_addr) = (mem.clk, mem.addr);
    }

    // If the trace length is not a power of two, we need to extend the trace to the
    // next power of two. The additional elements are filled with the last row
    // of the trace.
    pad_mem_trace(merged_trace)
}

#[cfg(test)]
mod tests {
    use im::hashmap::HashMap;
    use mozak_runner::elf::{Data, Program};
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::hash::hash_types::RichField;
    use plonky2::plonk::config::{GenericConfig, PoseidonGoldilocksConfig};

    use crate::memory::columns::{self as mem_cols, Memory};
    use crate::memory::test_utils::memory_trace_test_case;
    use crate::memory::trace::{OPCODE_LBU, OPCODE_SB};
    use crate::test_utils::inv;

    const D: usize = 2;
    type C = PoseidonGoldilocksConfig;
    type F = <C as GenericConfig<D>>::F;

    fn prep_table<F: RichField>(table: Vec<[u64; mem_cols::NUM_MEM_COLS]>) -> Vec<Memory<F>> {
        table
            .into_iter()
            .map(|row| row.into_iter().map(F::from_canonical_u64).collect())
            .collect()
    }

    fn expected_trace<F: RichField>() -> Vec<Memory<F>> {
        let sb = OPCODE_SB as u64;
        let lbu = OPCODE_LBU as u64;
        let inv = inv::<F>;
        #[rustfmt::skip]
        prep_table(vec![
            // is_executed  is_writable   is_init   addr  clk   op  value  diff_addr  diff_addr_inv  diff_clk
            [  1,                 1,        1,      100,  0,     0,    0,    100,     inv(100),            0],  // Memory Init: 100
            [  1,                 1,        0,      100,  0,    sb,  255,      0,           0,             0],  // Operations:  100
            [  1,                 1,        0,      100,  1,    lbu, 255,      0,           0,             1],  // Operations:  100
            [  1,                 1,        0,      100,  4,    sb,   10,      0,           0,             3],  // Operations:  100
            [  1,                 1,        0,      100,  5,    lbu,  10,      0,           0,             1],  // Operations:  100

            [  1,                 1,        1,      101,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 101
            [  1,                 1,        1,      102,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 102
            [  1,                 1,        1,      103,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 103

            [  1,                 1,        1,      200,  0,     0,    0,     97,     inv(97),             0],  // Memory Init: 200
            [  1,                 1,        0,      200,  2,    sb,   15,      0,           0,             2],  // Operations:  200
            [  1,                 1,        0,      200,  3,    lbu,  15,      0,           0,             1],  // Operations:  200

            [  1,                 1,        1,      201,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 201
            [  1,                 1,        1,      202,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 202
            [  1,                 1,        1,      203,  0,     0,    0,      1,      inv(1),             0],  // Memory Init: 203

            [  0,                 1,        1,      203,  0,     0,    0,      0,           0,             0],  // Padding
            [  0,                 1,        1,      203,  0,     0,    0,      0,           0,             0],  // Padding
        ])
    }

    // This test simulates the scenario of a set of instructions
    // which perform store byte (SB) and load byte unsigned (LBU) operations
    // to memory and then checks if the memory trace is generated correctly.
    #[test]
    fn generate_memory_trace() {
        let (program, record) = memory_trace_test_case(1);

        let trace = super::generate_memory_trace::<GoldilocksField>(&program, &record.executed);
        assert_eq!(trace, expected_trace());
    }

    #[test]
    fn generate_memory_trace_only_init() {
        let program = Program {
            ro_memory: Data(
                [(100, 5), (101, 6)]
                    .iter()
                    .cloned()
                    .collect::<HashMap<u32, u8>>(),
            ),
            rw_memory: Data(
                [(200, 7), (201, 8)]
                    .iter()
                    .cloned()
                    .collect::<HashMap<u32, u8>>(),
            ),
            ..Program::default()
        };

        let trace = super::generate_memory_trace::<F>(&program, &[]);

        let inv = inv::<F>;
        #[rustfmt::skip]
        assert_eq!(trace, prep_table(vec![
            // is_executed  is_writable   is_init   addr  clk   op  value  diff_addr  diff_addr_inv  diff_clk
            [  1,                 0,        1,      100,  0,    0,   5,    100,    inv(100),             0],
            [  1,                 0,        1,      101,  0,    0,   6,      1,           1,             0],
            [  1,                 1,        1,      200,  0,    0,   7,     99,     inv(99),             0],
            [  1,                 1,        1,      201,  0,    0,   8,      1,           1,             0],
        ]));
    }
}
