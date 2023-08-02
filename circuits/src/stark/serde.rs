use flexbuffers::{FlexbufferSerializer, Reader};
use plonky2::field::extension::Extendable;
use plonky2::hash::hash_types::RichField;
use plonky2::plonk::config::GenericConfig;
use serde::{Deserialize, Serialize};
use anyhow::Result;

use super::proof::AllProof;

impl<F: RichField + Extendable<D>, C: GenericConfig<D, F = F>, const D: usize> AllProof<F, C, D> {
    /// Serialize `AllProof` to flexbuffer.
    ///
    /// # Errors
    /// Errors if serialization fails.
    pub fn serialize_proof_to_flexbuffer(self) -> Result<FlexbufferSerializer> {
        let mut s = FlexbufferSerializer::new();
        self.serialize(&mut s)?;
        Ok(s)
    }

    /// Deserialize `AllProof` from flexbuffer.
    ///
    /// # Errors
    /// Errors if deserialization fails.
    pub fn deserialize_proof_from_flexbuffer(proof_bytes: &[u8]) -> Result<Self> {
        let r = Reader::get_root(proof_bytes)?;
        Ok(AllProof::deserialize(r)?)
    }
}

#[cfg(test)]
mod tests {

    use mozak_vm::test_utils::simple_test;
    use plonky2::util::timing::TimingTree;

    use crate::stark::proof::AllProof;
    use crate::stark::prover::prove;
    use crate::stark::verifier::verify_proof;
    use crate::test_utils::{standard_faster_config, C, D, F, S};
    #[test]
    fn test_serialization_deserialization() {
        let record = simple_test(0, &[], &[]);
        let stark = S::default();
        let config = standard_faster_config();

        let all_proof = prove::<F, C, D>(
            &record.executed,
            &stark,
            &config,
            &mut TimingTree::default(),
        )
        .unwrap();
        let s = all_proof.serialize_proof_to_flexbuffer().expect("serialization failed");
        let all_proof_deserialized =
            AllProof::<F, C, D>::deserialize_proof_from_flexbuffer(s.view()).expect("deserialization failed");
        verify_proof(stark, all_proof_deserialized, &config).unwrap();
    }
}
