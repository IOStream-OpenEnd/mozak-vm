use itertools::Itertools;
use plonky2::field::polynomial::PolynomialValues;
use plonky2::field::types::Field;
use plonky2::util::transpose;

pub fn trace_to_poly_values<F: Field, const COLUMNS: usize>(
    trace: [Vec<F>; COLUMNS],
) -> Vec<PolynomialValues<F>> {
    trace.into_iter().map(PolynomialValues::new).collect()
}

// TODO: rewrite or adapt from transpose in memory module.
/// A helper function to transpose a row-wise trace and put it in the format
/// that `prove` expects.
#[must_use]
pub fn trace_rows_to_poly_values<F: Field, Row: IntoIterator<Item = F>>(
    trace_rows: Vec<Row>,
) -> Vec<PolynomialValues<F>> {
    let trace_row_vecs = trace_rows
        .into_iter()
        .map(|row| row.into_iter().collect_vec())
        .collect_vec();
    let trace_col_vecs: Vec<Vec<F>> = transpose(&trace_row_vecs);
    trace_col_vecs
        .into_iter()
        .map(PolynomialValues::new)
        .collect()
}
