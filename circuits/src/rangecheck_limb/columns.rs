use plonky2::field::types::Field;

use crate::columns_view::{columns_view_impl, make_col_map};
use crate::cross_table_lookup::Column;

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq, Debug, Default)]
pub struct RangeCheckLimb<T> {
    // Column containing values 0..u8::MAX, with possible duplicates.
    pub element: T,
    // Filter to indicate a value to be range checked is not a dummy value.
    pub filter: T,
}
columns_view_impl!(RangeCheckLimb);
make_col_map!(RangeCheckLimb);

#[must_use]
pub fn data<F: Field>() -> Vec<Column<F>> { vec![Column::single(MAP.element)] }

/// Column for a binary filter to indicate whether a row in the
/// [`RangeCheckTable`](crate::cross_table_lookup::RangeCheckTable).
/// contains a non-dummy value to be range checked.
#[must_use]
pub fn filter<F: Field>() -> Column<F> { Column::single(MAP.filter) }