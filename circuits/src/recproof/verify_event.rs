//! Subcircuits for proving events can be summarized as a partial object.

use anyhow::Result;
use plonky2::field::extension::Extendable;
use plonky2::hash::hash_types::{HashOut, RichField};
use plonky2::iop::target::Target;
use plonky2::iop::witness::{PartialWitness, WitnessWrite};
use plonky2::plonk::circuit_builder::CircuitBuilder;
use plonky2::plonk::circuit_data::{CircuitConfig, CircuitData};
use plonky2::plonk::config::{AlgebraicHasher, GenericConfig};
use plonky2::plonk::proof::ProofWithPublicInputs;

use super::{byte_wise_hash_event, hash_event, propagate, unbounded, unpruned, Event};

pub struct LeafTargets {
    /// The event type
    pub event_ty: Target,

    /// The event address
    pub event_address: Target,

    /// The event value
    pub event_value: [Target; 4],
}

pub struct LeafCircuit<F, C, const D: usize>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>, {
    /// The recursion subcircuit
    pub unbounded: unbounded::LeafSubCircuit,

    /// The rc-style merkle hash of all event fields
    pub hash: unpruned::LeafSubCircuit,

    /// The vm-style merkle hash of all event fields
    pub vm_hash: unpruned::LeafSubCircuit,

    /// The owner of this event propagated throughout this tree
    pub event_owner: propagate::LeafSubCircuit<4>,

    /// The other event fields
    pub targets: LeafTargets,

    pub circuit: CircuitData<F, C, D>,
}

impl<F, C, const D: usize> LeafCircuit<F, C, D>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>,
    <C as GenericConfig<D>>::Hasher: AlgebraicHasher<F>,
{
    #[must_use]
    pub fn new(circuit_config: &CircuitConfig) -> Self {
        let mut builder = CircuitBuilder::<F, D>::new(circuit_config.clone());

        let unbounded_inputs = unbounded::SubCircuitInputs::default(&mut builder);
        let hash_inputs = unpruned::SubCircuitInputs::default(&mut builder);
        let vm_hash_inputs = unpruned::SubCircuitInputs::default(&mut builder);
        let event_owner_inputs = propagate::SubCircuitInputs::<4>::default(&mut builder);

        let unbounded_targets = unbounded_inputs.build_leaf::<F, C, D>(&mut builder);
        let hash_targets = hash_inputs.build_leaf(&mut builder);
        let vm_hash_targets = vm_hash_inputs.build_leaf(&mut builder);
        let event_owner_targets = event_owner_inputs.build_leaf(&mut builder);

        let targets = LeafTargets {
            event_ty: builder.add_virtual_target(),
            event_address: builder.add_virtual_target(),
            event_value: builder.add_virtual_target_arr::<4>(),
        };

        let event_hash = hash_event(
            &mut builder,
            event_owner_targets.inputs.values,
            targets.event_ty,
            targets.event_address,
            targets.event_value,
        );
        let event_vm_hash = byte_wise_hash_event(
            &mut builder,
            event_owner_targets.inputs.values,
            targets.event_ty,
            targets.event_address,
            targets.event_value,
        );

        builder.connect_hashes(hash_targets.inputs.unpruned_hash, event_hash);
        builder.connect_hashes(vm_hash_targets.inputs.unpruned_hash, event_vm_hash);

        let circuit = builder.build();

        let public_inputs = &circuit.prover_only.public_inputs;
        let unbounded = unbounded_targets.build(public_inputs);
        let hash = hash_targets.build(public_inputs);
        let vm_hash = vm_hash_targets.build(public_inputs);
        let event_owner = event_owner_targets.build(public_inputs);

        Self {
            unbounded,
            hash,
            vm_hash,
            event_owner,
            targets,
            circuit,
        }
    }

    /// `hash` only needs to be provided to check externally, otherwise it will
    /// be calculated
    pub fn prove(
        &self,
        event: Event<F>,
        hash: Option<HashOut<F>>,
        vm_hash: Option<HashOut<F>>,
        branch: &BranchCircuit<F, C, D>,
    ) -> Result<ProofWithPublicInputs<F, C, D>> {
        let mut inputs = PartialWitness::new();
        self.unbounded.set_witness(&mut inputs, &branch.circuit);
        if let Some(hash) = hash {
            self.hash.set_witness(&mut inputs, hash);
        }
        if let Some(vm_hash) = vm_hash {
            self.vm_hash.set_witness(&mut inputs, vm_hash);
        }
        self.event_owner.set_witness(&mut inputs, event.owner);
        inputs.set_target(self.targets.event_ty, F::from_canonical_u8(event.ty as u8));
        inputs.set_target(
            self.targets.event_address,
            F::from_canonical_u64(event.address),
        );
        inputs.set_target_arr(&self.targets.event_value, &event.value);
        self.circuit.prove(inputs)
    }
}

pub struct BranchCircuit<F, C, const D: usize>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>, {
    pub unbounded: unbounded::BranchSubCircuit<D>,
    /// The merkle hash of all events
    pub hash: unpruned::BranchSubCircuit,

    /// The vm-style merkle hash of all events
    pub vm_hash: unpruned::BranchSubCircuit,

    /// The owner of the events propagated throughout this tree
    pub event_owner: propagate::BranchSubCircuit<4>,

    pub circuit: CircuitData<F, C, D>,
}

impl<F, C, const D: usize> BranchCircuit<F, C, D>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>,
    <C as GenericConfig<D>>::Hasher: AlgebraicHasher<F>,
{
    #[must_use]
    pub fn new(circuit_config: &CircuitConfig, leaf: &LeafCircuit<F, C, D>) -> Self {
        let mut builder = CircuitBuilder::<F, D>::new(circuit_config.clone());

        let unbounded_inputs = unbounded::SubCircuitInputs::default(&mut builder);
        let hash_inputs = unpruned::SubCircuitInputs::default(&mut builder);
        let vm_hash_inputs = unpruned::SubCircuitInputs::default(&mut builder);
        let event_owner_inputs = propagate::SubCircuitInputs::<4>::default(&mut builder);

        let unbounded_targets =
            unbounded_inputs.build_branch(&mut builder, &leaf.unbounded, &leaf.circuit);
        let hash_targets = hash_inputs.build_branch(
            &mut builder,
            &leaf.hash.indices,
            &unbounded_targets.left_proof,
            &unbounded_targets.right_proof,
            false,
        );
        let vm_hash_targets = vm_hash_inputs.build_branch(
            &mut builder,
            &leaf.vm_hash.indices,
            &unbounded_targets.left_proof,
            &unbounded_targets.right_proof,
            true,
        );
        let event_owner_targets = event_owner_inputs.build_branch(
            &mut builder,
            &leaf.event_owner.indices,
            &unbounded_targets.left_proof,
            &unbounded_targets.right_proof,
        );

        let circuit = builder.build();

        let public_inputs = &circuit.prover_only.public_inputs;
        let unbounded = unbounded_targets.build(&leaf.unbounded, public_inputs);
        let hash = hash_targets.build(&leaf.hash.indices, public_inputs);
        let vm_hash = vm_hash_targets.build(&leaf.vm_hash.indices, public_inputs);
        let event_owner = event_owner_targets.build(&leaf.event_owner.indices, public_inputs);

        Self {
            unbounded,
            hash,
            vm_hash,
            event_owner,
            circuit,
        }
    }

    /// `hash` `vm_hash` and `event_owner` only need to be provided to check
    /// externally, otherwise they will be calculated
    #[allow(clippy::too_many_arguments)]
    pub fn prove(
        &self,
        hash: Option<HashOut<F>>,
        vm_hash: Option<HashOut<F>>,
        event_owner: Option<[F; 4]>,
        left_is_leaf: bool,
        right_is_leaf: bool,
        left_proof: &ProofWithPublicInputs<F, C, D>,
        right_proof: &ProofWithPublicInputs<F, C, D>,
    ) -> Result<ProofWithPublicInputs<F, C, D>> {
        let mut inputs = PartialWitness::new();
        self.unbounded.set_witness(
            &mut inputs,
            left_is_leaf,
            right_is_leaf,
            left_proof,
            right_proof,
        );
        if let Some(hash) = hash {
            self.hash.set_witness(&mut inputs, hash);
        }
        if let Some(vm_hash) = vm_hash {
            self.vm_hash.set_witness(&mut inputs, vm_hash);
        }
        if let Some(event_owner) = event_owner {
            self.event_owner.set_witness(&mut inputs, event_owner);
        }
        self.circuit.prove(inputs)
    }
}

#[cfg(test)]
mod test {
    use std::panic::catch_unwind;

    use itertools::{chain, Itertools};
    use lazy_static::lazy_static;
    use plonky2::field::types::Field;
    use plonky2::hash::hash_types::NUM_HASH_OUT_ELTS;
    use plonky2::hash::poseidon2::Poseidon2Hash;
    use plonky2::plonk::config::Hasher;

    use super::*;
    use crate::recproof::EventType;
    use crate::test_utils::{fast_test_circuit_config, hash_branch, C, D, F};

    const CONFIG: CircuitConfig = fast_test_circuit_config();

    lazy_static! {
        static ref LEAF: LeafCircuit<F, C, D> = LeafCircuit::new(&CONFIG);
        static ref BRANCH: BranchCircuit<F, C, D> = BranchCircuit::new(&CONFIG, &LEAF);
    }

    fn hash_branch_bytes<F: RichField>(left: &HashOut<F>, right: &HashOut<F>) -> HashOut<F> {
        let bytes = chain!(left.elements, right.elements)
            .flat_map(|v| v.to_canonical_u64().to_le_bytes())
            .map(|v| F::from_canonical_u8(v))
            .collect_vec();
        Poseidon2Hash::hash_no_pad(&bytes)
    }

    fn assert_hash(h: HashOut<F>, v: [u64; NUM_HASH_OUT_ELTS]) {
        assert_eq!(h.elements, v.map(F::from_canonical_u64));
    }

    #[allow(clippy::unreadable_literal)]
    fn verify_simple_hashes(
        read_0_byte_hash: HashOut<F>,
        write_1_byte_hash: HashOut<F>,
        write_2_byte_hash: HashOut<F>,
        branch_1_bytes_hash: HashOut<F>,
        branch_2_bytes_hash: HashOut<F>,
    ) {
        const READ_0_HASH: [u64; NUM_HASH_OUT_ELTS] = [
            7272290939186032751,
            8185818005188304227,
            17555306369107993266,
            17187284268557234321,
        ];
        const WRITE_1_HASH: [u64; NUM_HASH_OUT_ELTS] = [
            11469795294276139037,
            799622748573506082,
            15272809121316752941,
            7142640452443475716,
        ];
        const WRITE_2_HASH: [u64; NUM_HASH_OUT_ELTS] = [
            1484423020241144842,
            17207848040428508675,
            7995793996020726058,
            4658801606188332384,
        ];
        const BRANCH_1_HASH: [u64; NUM_HASH_OUT_ELTS] = [
            16758566829994364981,
            15311795646108582705,
            12773152691662485878,
            2551708493265210224,
        ];
        const BRANCH_2_HASH: [u64; NUM_HASH_OUT_ELTS] = [
            8577138257922146843,
            5112874340235798754,
            4121828782781403483,
            12250937462246573507,
        ];

        assert_hash(read_0_byte_hash, READ_0_HASH);
        assert_hash(write_1_byte_hash, WRITE_1_HASH);
        assert_hash(write_2_byte_hash, WRITE_2_HASH);
        assert_hash(branch_1_bytes_hash, BRANCH_1_HASH);
        assert_hash(branch_2_bytes_hash, BRANCH_2_HASH);
    }

    #[test]
    fn verify_simple() -> Result<()> {
        let program_hash_1 = [4, 8, 15, 16].map(F::from_canonical_u64);

        let zero_val = [F::ZERO; 4];
        let non_zero_val_1 = [3, 1, 4, 15].map(F::from_canonical_u64);
        let non_zero_val_2 = [1, 6, 180, 33].map(F::from_canonical_u64);

        // Duplicate or conflicting events are actually fine as far as this circuit
        // cares
        let read_0 = Event {
            address: 42,
            owner: program_hash_1,
            ty: EventType::Read,
            value: zero_val,
        };
        let write_1 = Event {
            address: 42,
            owner: program_hash_1,
            ty: EventType::Write,
            value: non_zero_val_1,
        };
        let write_2 = Event {
            address: 42,
            owner: program_hash_1,
            ty: EventType::Write,
            value: non_zero_val_2,
        };
        let read_0_hash = read_0.hash();
        let write_1_hash = write_1.hash();
        let write_2_hash = write_2.hash();
        let read_0_byte_hash = read_0.byte_wise_hash();
        let write_1_byte_hash = write_1.byte_wise_hash();
        let write_2_byte_hash = write_2.byte_wise_hash();

        // Read zero
        let read_proof = LEAF.prove(
            Event {
                address: 42,
                ty: EventType::Read,
                owner: program_hash_1,
                value: zero_val,
            },
            Some(read_0_hash),
            Some(read_0_byte_hash),
            &BRANCH,
        )?;
        LEAF.circuit.verify(read_proof.clone())?;

        // Write pi
        let write_proof_1 = LEAF.prove(
            Event {
                address: 42,
                ty: EventType::Write,
                owner: program_hash_1,
                value: non_zero_val_1,
            },
            Some(write_1_hash),
            Some(write_1_byte_hash),
            &BRANCH,
        )?;
        LEAF.circuit.verify(write_proof_1.clone())?;

        // Write phi (this is legal for this stage, but illegal generally as a double
        // write)
        let write_proof_2 = LEAF.prove(
            Event {
                address: 42,
                ty: EventType::Write,
                owner: program_hash_1,
                value: non_zero_val_2,
            },
            Some(write_2_hash),
            Some(write_2_byte_hash),
            &BRANCH,
        )?;
        LEAF.circuit.verify(write_proof_2.clone())?;

        let branch_1_hash = hash_branch(&write_1_hash, &write_2_hash);
        let branch_2_hash = hash_branch(&read_0_hash, &branch_1_hash);
        let branch_1_bytes_hash = hash_branch_bytes(&write_1_byte_hash, &write_2_byte_hash);
        let branch_2_bytes_hash = hash_branch_bytes(&read_0_byte_hash, &branch_1_bytes_hash);

        // Combine writes
        let branch_proof_1 = BRANCH.prove(
            Some(branch_1_hash),
            Some(branch_1_bytes_hash),
            Some(program_hash_1),
            true,
            true,
            &write_proof_1,
            &write_proof_2,
        )?;
        BRANCH.circuit.verify(branch_proof_1.clone())?;

        // Combine with reads
        let branch_proof_2 = BRANCH.prove(
            Some(branch_2_hash),
            Some(branch_2_bytes_hash),
            Some(program_hash_1),
            true,
            false,
            &read_proof,
            &branch_proof_1,
        )?;
        BRANCH.circuit.verify(branch_proof_2)?;

        verify_simple_hashes(
            read_0_byte_hash,
            write_1_byte_hash,
            write_2_byte_hash,
            branch_1_bytes_hash,
            branch_2_bytes_hash,
        );

        Ok(())
    }

    #[test]
    #[should_panic(expected = "was set twice with different values")]
    fn bad_leaf_hash() {
        let (read_1, read_0_hash, read_0_byte_hash) = catch_unwind(|| {
            let program_hash_1 = [4, 8, 15, 16].map(F::from_canonical_u64);
            let program_hash_2 = [2, 3, 4, 2].map(F::from_canonical_u64);

            let zero_val = [F::ZERO; 4];

            let read_0 = Event {
                address: 42,
                owner: program_hash_1,
                ty: EventType::Read,
                value: zero_val,
            };
            let read_1 = Event {
                address: 42,
                owner: program_hash_2,
                ty: EventType::Read,
                value: zero_val,
            };

            let read_0_hash = read_0.hash();
            let read_0_byte_hash = read_0.byte_wise_hash();
            (read_1, read_0_hash, read_0_byte_hash)
        })
        .expect("shouldn't fail");

        // Fail to prove with mismatched hashes
        LEAF.prove(read_1, Some(read_0_hash), Some(read_0_byte_hash), &BRANCH)
            .unwrap();
    }

    #[test]
    #[should_panic(expected = "was set twice with different values")]
    fn bad_program_match() {
        let (program_hash_1, branch_1_hash, branch_1_bytes_hash, read_proof_1, read_proof_2) =
            catch_unwind(|| {
                let program_hash_1 = [4, 8, 15, 16].map(F::from_canonical_u64);
                let program_hash_2 = [2, 3, 4, 2].map(F::from_canonical_u64);

                let zero_val = [F::ZERO; 4];

                // Read events from two different programs
                let read_0 = Event {
                    address: 42,
                    owner: program_hash_1,
                    ty: EventType::Read,
                    value: zero_val,
                };
                let read_1 = Event {
                    address: 42,
                    owner: program_hash_2,
                    ty: EventType::Read,
                    value: zero_val,
                };

                let read_0_hash = read_0.hash();
                let read_1_hash = read_1.hash();
                let read_0_byte_hash = read_0.byte_wise_hash();
                let read_1_byte_hash = read_1.byte_wise_hash();

                // Read zero
                let read_proof_1 = LEAF
                    .prove(read_0, Some(read_0_hash), Some(read_0_byte_hash), &BRANCH)
                    .unwrap();
                LEAF.circuit.verify(read_proof_1.clone()).unwrap();

                let read_proof_2 = LEAF
                    .prove(read_1, Some(read_1_hash), Some(read_1_byte_hash), &BRANCH)
                    .unwrap();
                LEAF.circuit.verify(read_proof_2.clone()).unwrap();

                // Combine reads
                let branch_1_hash = hash_branch(&read_0_hash, &read_1_hash);
                let branch_1_bytes_hash = hash_branch_bytes(&read_0_byte_hash, &read_1_byte_hash);
                (
                    program_hash_1,
                    branch_1_hash,
                    branch_1_bytes_hash,
                    read_proof_1,
                    read_proof_2,
                )
            })
            .expect("shouldn't fail");

        // Fail to prove with mismatched program hashes between branches
        // This tree requires all events are from the same program
        BRANCH
            .prove(
                Some(branch_1_hash),
                Some(branch_1_bytes_hash),
                Some(program_hash_1),
                true,
                true,
                &read_proof_1,
                &read_proof_2,
            )
            .unwrap();
    }
}