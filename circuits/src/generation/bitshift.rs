use itertools::Itertools;
use plonky2::hash::hash_types::RichField;

use crate::bitshift::columns::ShiftAmountView;
use crate::cpu::columns::CpuColumnsView;

fn filter_shift_trace<F: RichField>(
    step_rows: &[CpuColumnsView<F>],
) -> impl Iterator<Item = u64> + '_ {
    step_rows.iter().filter_map(|row| {
        (row.inst.ops.ops_that_shift().into_iter().sum::<F>() != F::ZERO)
            .then_some(row.bitshift.amount.to_noncanonical_u64())
    })
}

pub fn pad_trace<Row: Copy>(mut trace: Vec<Row>, default: Row) -> Vec<Row> {
    trace.resize(trace.len().next_power_of_two(), default);
    trace
}

#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn generate_shift_amount_trace<F: RichField>(
    cpu_trace: &[CpuColumnsView<F>],
) -> Vec<ShiftAmountView<F>> {
    pad_trace(
        filter_shift_trace(cpu_trace)
            .sorted()
            .merge_join_by(0..32, u64::cmp)
            .map(|dummy_or_executed| {
                ShiftAmountView {
                    is_executed: dummy_or_executed.is_left().into(),
                    executed: dummy_or_executed.into_left().into(),
                }
                .map(F::from_canonical_u64)
            })
            .collect(),
        ShiftAmountView {
            is_executed: false.into(),
            executed: 31_u64.into(),
        }
        .map(F::from_canonical_u64),
    )
}
