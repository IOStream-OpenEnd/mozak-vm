use plonky2::field::extension::FieldExtension;
use plonky2::field::packed::PackedField;
use plonky2::{field::extension::Extendable, hash::hash_types::RichField};

use super::config::StarkConfig;
use super::constraint_consumer::ConstraintConsumer;
use super::vars::StarkEvaluationVars;

/// A STARK System.
pub trait Stark<F: RichField + Extendable<D>, const D: usize>: Sync {
    /// The total number of columns in the trace.
    const COLUMNS: usize;

    fn eval_packed_generic<FE, P, const D2: usize>(
        &self,
        vars: StarkEvaluationVars<FE, P, { Self::COLUMNS }>,
        yield_constr: &mut ConstraintConsumer<P>,
    ) where
        FE: FieldExtension<D2, BaseField = F>,
        P: PackedField<Scalar = FE>;

    fn constraint_degree(&self) -> usize;
    fn quotient_degree_factor(&self) -> usize {
        1.max(self.constraint_degree() - 1)
    }
    fn num_quotient_poly(&self, config: &StarkConfig) -> usize {
        self.quotient_degree_factor() * config.num_challenges
    }
}
