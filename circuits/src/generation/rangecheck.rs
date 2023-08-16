use std::borrow::Borrow;
use std::ops::Index;

use itertools::Itertools;
use plonky2::hash::hash_types::RichField;

use crate::cpu::columns::CpuState;
use crate::lookup::permute_cols;
use crate::memory::columns::Memory;
use crate::rangecheck::columns::{self, RangeCheckColumnsView, MAP};
use crate::stark::mozak_stark::{Lookups, RangecheckCpuTable, Table, TableKind};

pub(crate) const RANGE_CHECK_U16_SIZE: usize = 1 << 16;

/// Pad the rangecheck trace table to the size of 2^k rows in
/// preparation for the Halo2 lookup argument.
///
/// Note that by right the column to be checked (A) and the fixed column (S)
/// have to be extended by dummy values known to be in the fixed column if they
/// are not of size 2^k, but because our fixed column is a range from 0..2^16-1,
/// initializing our trace to all [`F::ZERO`]s takes care of this step by
/// default.
#[must_use]
fn pad_rc_trace<F: RichField>(mut trace: Vec<Vec<F>>) -> Vec<Vec<F>> {
    let len = trace[0].len().max(RANGE_CHECK_U16_SIZE).next_power_of_two();

    trace.iter_mut().for_each(move |c| c.resize(len, F::ZERO));

    trace
}

/// Converts a u32 into 2 u16 limbs represented in [`RichField`].
fn limbs_from_u32<F: RichField>(value: u32) -> (F, F) {
    (
        F::from_noncanonical_u64((value >> 16).into()),
        F::from_noncanonical_u64((value & 0xffff).into()),
    )
}

fn push_rangecheck_row<F: RichField>(
    trace: &mut [Vec<F>],
    rangecheck_row: &[F; columns::NUM_RC_COLS],
) {
    for (i, col) in rangecheck_row.iter().enumerate() {
        trace[i].push(*col);
    }
}

#[allow(clippy::missing_panics_doc)]
pub fn extract<'a, F: RichField, V>(trace: &[V], looking_table: &Table<F>) -> Vec<F>
where
    V: Index<usize, Output = F> + 'a, {
    if let [column] = &looking_table.columns[..] {
        trace
            .iter()
            .circular_tuple_windows()
            .filter_map(|(prev_row, row)| {
                looking_table
                    .filter_column
                    .eval(prev_row, row)
                    .is_one()
                    .then(|| column.eval(prev_row, row))
            })
            .collect()
    } else {
        panic!("Can only range check single values, not tuples.")
    }
}

/// Generates a trace table for range checks, used in building a
/// `RangeCheckStark` proof.
///
/// # Panics
///
/// Panics if:
/// 1. conversion of u32 values to u16 limbs,
/// 2. trace width does not match the number of columns,
/// 3. attempting to range check tuples instead of single values.
#[must_use]
pub fn generate_rangecheck_trace<F: RichField>(
    cpu_trace: &[CpuState<F>],
    memory_trace: &[Memory<F>],
) -> [Vec<F>; columns::NUM_RC_COLS] {
    let mut trace: Vec<Vec<F>> = vec![vec![]; columns::NUM_RC_COLS];

    for looking_table in RangecheckCpuTable::lookups().looking_tables {
        let values = match looking_table.kind {
            TableKind::Cpu => extract(cpu_trace, &looking_table),
            TableKind::Memory => extract(memory_trace, &looking_table),
            other => unimplemented!("Can't range check {other:#?} tables"),
        };

        for val in values {
            let (limb_hi, limb_lo) = limbs_from_u32(
                u32::try_from(val.to_canonical_u64()).expect("casting value to u32 should succeed"),
            );
            let rangecheck_row = RangeCheckColumnsView {
                val,
                limb_lo,
                limb_hi,
                cpu_filter: F::ONE,
                ..Default::default()
            };
            push_rangecheck_row(&mut trace, rangecheck_row.borrow());
        }
    }

    // Pad our trace to max(RANGE_CHECK_U16_SIZE, trace[0].len())
    trace = pad_rc_trace(trace);

    // Here, we generate fixed columns for the table, used in inner table lookups.
    // We are interested in range checking 16-bit values, hence we populate with
    // values 0, 1, .., 2^16 - 1.
    trace[MAP.fixed_range_check_u16] = (0..RANGE_CHECK_U16_SIZE as u64)
        .map(F::from_noncanonical_u64)
        .collect();
    let num_rows = trace[MAP.val].len();
    trace[MAP.fixed_range_check_u16].resize(num_rows, F::from_canonical_u64(u64::from(u16::MAX)));

    // This permutation is done in accordance to the [Halo2 lookup argument
    // spec](https://zcash.github.io/halo2/design/proving-system/lookup.html)
    let (col_input_permuted, col_table_permuted) =
        permute_cols(&trace[MAP.limb_lo], &trace[MAP.fixed_range_check_u16]);

    // We need a column for the lower limb.
    trace[MAP.limb_lo_permuted] = col_input_permuted;
    trace[MAP.fixed_range_check_u16_permuted_lo] = col_table_permuted;

    let (col_input_permuted, col_table_permuted) =
        permute_cols(&trace[MAP.limb_hi], &trace[MAP.fixed_range_check_u16]);

    // And we also need a column for the upper limb.
    trace[MAP.limb_hi_permuted] = col_input_permuted;
    trace[MAP.fixed_range_check_u16_permuted_hi] = col_table_permuted;

    trace.try_into().unwrap_or_else(|v: Vec<Vec<F>>| {
        panic!(
            "Expected a Vec of length {} but it was {}",
            columns::NUM_RC_COLS,
            v.len()
        )
    })
}

#[cfg(test)]
mod tests {
    use mozak_vm::instruction::{Args, Instruction, Op};
    use mozak_vm::test_utils::simple_test_code;
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::field::types::Field;

    use super::*;
    use crate::generation::cpu::generate_cpu_trace;
    use crate::generation::memory::generate_memory_trace;

    #[test]
    fn test_add_instruction_inserts_rangecheck() {
        type F = GoldilocksField;
        let (program, record) = simple_test_code(
            &[Instruction {
                op: Op::ADD,
                args: Args {
                    rd: 5,
                    rs1: 6,
                    rs2: 7,
                    ..Args::default()
                },
            }],
            // Use values that would become limbs later
            &[],
            &[(6, 0xffff), (7, 0xffff)],
        );

        let cpu_rows = generate_cpu_trace::<F>(&program, &record);
        let memory_rows = generate_memory_trace::<F>(&program, &record.executed);
        let trace = generate_rangecheck_trace::<F>(&cpu_rows, &memory_rows);

        // Check values that we are interested in
        assert_eq!(trace[MAP.cpu_filter][0], F::ONE);
        assert_eq!(trace[MAP.cpu_filter][1], F::ONE);
        assert_eq!(trace[MAP.val][1], GoldilocksField(0x0001_fffe));
        assert_eq!(trace[MAP.val][0], GoldilocksField(93));
        assert_eq!(trace[MAP.limb_hi][1], GoldilocksField(0x0001));
        assert_eq!(trace[MAP.limb_lo][1], GoldilocksField(0xfffe));
        assert_eq!(trace[MAP.limb_lo][0], GoldilocksField(93));

        // Ensure rest of trace is zeroed out
        for cpu_filter in &trace[MAP.cpu_filter][2..] {
            assert_eq!(cpu_filter, &F::ZERO);
        }
        for value in &trace[MAP.val][2..] {
            assert_eq!(value, &F::ZERO);
        }
        for limb_hi in &trace[MAP.limb_hi][2..] {
            assert_eq!(limb_hi, &F::ZERO);
        }
        for limb_lo in &trace[MAP.limb_lo][2..] {
            assert_eq!(limb_lo, &F::ZERO);
        }
    }
}
