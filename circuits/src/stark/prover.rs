#![allow(clippy::too_many_lines)]

use std::fmt::Display;

use anyhow::{ensure, Result};
use itertools::Itertools;
use log::Level::Debug;
use log::{debug, log_enabled};
use mozak_runner::elf::Program;
use mozak_runner::vm::ExecutionRecord;
use plonky2::field::extension::Extendable;
use plonky2::field::packable::Packable;
use plonky2::field::polynomial::PolynomialValues;
use plonky2::field::types::Field;
use plonky2::fri::oracle::PolynomialBatch;
use plonky2::hash::hash_types::RichField;
use plonky2::iop::challenger::Challenger;
use plonky2::plonk::config::GenericConfig;
use plonky2::timed;
use plonky2::util::log2_strict;
use plonky2::util::timing::TimingTree;
#[allow(clippy::wildcard_imports)]
use plonky2_maybe_rayon::*;
use starky::config::StarkConfig;
use starky::stark::{LookupConfig, Stark};

use super::mozak_stark::{MozakStark, TableKind, TableKindArray, TableKindSetBuilder};
use super::proof::{AllProof, StarkOpeningSet, StarkProof};
use crate::cross_table_lookup::ctl_utils::debug_ctl;
use crate::cross_table_lookup::{cross_table_lookup_data, CtlData};
use crate::generation::{debug_traces, generate_traces};
use crate::public_sub_table::public_sub_table_data_and_values;
use crate::stark::mozak_stark::{all_starks, PublicInputs};
use crate::stark::permutation::challenge::GrandProductChallengeTrait;
use crate::stark::poly::compute_quotient_polys;

/// Prove the execution of a given [Program]
///
/// ## Parameters
/// `program`: A serialized ELF Program
/// `record`: Non-constrained execution trace generated by the runner
/// `mozak_stark`: Mozak-VM Gadgets
/// `config`: Stark and FRI security configurations
/// `public_inputs`: Public Inputs to the Circuit
/// `timing`: Profiling tool
pub fn prove<F, C, const D: usize>(
    program: &Program,
    record: &ExecutionRecord<F>,
    mozak_stark: &MozakStark<F, D>,
    config: &StarkConfig,
    public_inputs: PublicInputs<F>,
    timing: &mut TimingTree,
) -> Result<AllProof<F, C, D>>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>, {
    debug!("Starting Prove");
    let traces_poly_values = generate_traces(program, record);
    if mozak_stark.debug || std::env::var("MOZAK_STARK_DEBUG").is_ok() {
        debug_traces(&traces_poly_values, mozak_stark, &public_inputs);
        debug_ctl(&traces_poly_values, mozak_stark);
    }
    prove_with_traces(
        mozak_stark,
        config,
        public_inputs,
        &traces_poly_values,
        timing,
    )
}

/// Given the traces generated from [`generate_traces`], prove a [`MozakStark`].
///
/// # Errors
/// Errors if proving fails.
pub fn prove_with_traces<F, C, const D: usize>(
    mozak_stark: &MozakStark<F, D>,
    config: &StarkConfig,
    public_inputs: PublicInputs<F>,
    traces_poly_values: &TableKindArray<Vec<PolynomialValues<F>>>,
    timing: &mut TimingTree,
) -> Result<AllProof<F, C, D>>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>, {
    let rate_bits = config.fri_config.rate_bits;
    let cap_height = config.fri_config.cap_height;

    let trace_commitments = timed!(
        timing,
        "Compute trace commitments for each table",
        traces_poly_values
            .clone()
            .with_kind()
            .map(|(trace, table)| {
                timed!(
                    timing,
                    &format!("compute trace commitment for {table:?}"),
                    PolynomialBatch::<F, C, D>::from_values(
                        trace.clone(),
                        rate_bits,
                        false,
                        cap_height,
                        timing,
                        None,
                    )
                )
            })
    );

    let trace_caps = trace_commitments
        .each_ref()
        .map(|c| c.merkle_tree.cap.clone());
    // Add trace commitments to the challenger entropy pool.
    let mut challenger = Challenger::<F, C::Hasher>::new();
    for cap in &trace_caps {
        challenger.observe_cap(cap);
    }

    let ctl_challenges = challenger.get_grand_product_challenge_set(config.num_challenges);
    let ctl_data_per_table = timed!(
        timing,
        "Compute CTL data for each table",
        cross_table_lookup_data::<F, D>(
            traces_poly_values,
            &mozak_stark.cross_table_lookups,
            &ctl_challenges
        )
    );

    let (public_sub_table_data_per_table, public_sub_table_values) =
        public_sub_table_data_and_values::<F, D>(
            traces_poly_values,
            &mozak_stark.public_sub_tables,
            &ctl_challenges,
        );

    let proofs = timed!(
        timing,
        "compute all proofs given commitments",
        prove_with_commitments(
            mozak_stark,
            config,
            &public_inputs,
            traces_poly_values,
            &trace_commitments,
            &ctl_data_per_table,
            &public_sub_table_data_per_table,
            &mut challenger,
            timing
        )?
    );

    let program_rom_trace_cap = trace_caps[TableKind::Program].clone();
    let elf_memory_init_trace_cap = trace_caps[TableKind::ElfMemoryInit].clone();
    let mozak_memory_init_trace_cap = trace_caps[TableKind::MozakMemoryInit].clone();
    if log_enabled!(Debug) {
        timing.print();
    }
    Ok(AllProof {
        proofs,
        program_rom_trace_cap,
        elf_memory_init_trace_cap,
        mozak_memory_init_trace_cap,
        public_inputs,
        public_sub_table_values,
    })
}

/// Compute proof for a single STARK table, with lookup data.
///
/// # Errors
/// Errors if FRI parameters are wrongly configured, or if
/// there are no z polys, or if our
/// opening points are in our subgroup `H`,
#[allow(clippy::too_many_arguments)]
pub(crate) fn prove_single_table<F, C, S, const D: usize>(
    stark: &S,
    config: &StarkConfig,
    trace_poly_values: &[PolynomialValues<F>],
    trace_commitment: &PolynomialBatch<F, C, D>,
    public_inputs: &[F],
    ctl_data: &CtlData<F>,
    public_sub_table_data: &CtlData<F>,
    challenger: &mut Challenger<F, C::Hasher>,
    timing: &mut TimingTree,
) -> Result<StarkProof<F, C, D>>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>,
    S: Stark<F, D> + Display, {
    let degree = trace_poly_values[0].len();
    let degree_bits = log2_strict(degree);
    let fri_params = config.fri_params(degree_bits);
    let rate_bits = config.fri_config.rate_bits;
    let cap_height = config.fri_config.cap_height;
    assert!(
        fri_params.total_arities() <= degree_bits + rate_bits - cap_height,
        "FRI total reduction arity is too large.",
    );

    let z_poly_public_sub_table = public_sub_table_data.z_polys();

    // commit to both z poly of ctl and open public
    let z_polys = vec![ctl_data.z_polys(), z_poly_public_sub_table]
        .into_iter()
        .flatten()
        .collect_vec();
    // TODO(Matthias): make the code work with empty z_polys, too.
    assert!(!z_polys.is_empty(), "No CTL? {stark}");

    let ctl_zs_commitment = timed!(
        timing,
        format!("{stark}: compute Zs commitment").as_str(),
        PolynomialBatch::from_values(
            z_polys,
            rate_bits,
            false,
            config.fri_config.cap_height,
            timing,
            None,
        )
    );
    let ctl_zs_cap = ctl_zs_commitment.merkle_tree.cap.clone();
    challenger.observe_cap(&ctl_zs_cap);

    let alphas = challenger.get_n_challenges(config.num_challenges);
    let quotient_polys = timed!(
        timing,
        format!("{stark}: compute quotient polynomial").as_str(),
        compute_quotient_polys::<F, <F as Packable>::Packing, C, S, D>(
            stark,
            trace_commitment,
            &ctl_zs_commitment,
            public_inputs,
            ctl_data,
            public_sub_table_data,
            &alphas,
            degree_bits,
            config,
        )
    );

    let all_quotient_chunks = timed!(
        timing,
        format!("{stark}: split quotient polynomial").as_str(),
        quotient_polys
            .into_par_iter()
            .flat_map(|mut quotient_poly| {
                quotient_poly
                    .trim_to_len(degree * stark.quotient_degree_factor())
                    .expect(
                        "Quotient has failed, the vanishing polynomial is not divisible by Z_H",
                    );
                // Split quotient into degree-n chunks.
                quotient_poly.chunks(degree)
            })
            .collect()
    );
    let quotient_commitment = timed!(
        timing,
        format!("{stark}: compute quotient commitment").as_str(),
        PolynomialBatch::from_coeffs(
            all_quotient_chunks,
            rate_bits,
            false,
            config.fri_config.cap_height,
            timing,
            None,
        )
    );
    let quotient_polys_cap = quotient_commitment.merkle_tree.cap.clone();
    challenger.observe_cap(&quotient_polys_cap);

    let zeta = challenger.get_extension_challenge::<D>();
    // To avoid leaking witness data, we want to ensure that our opening locations,
    // `zeta` and `g * zeta`, are not in our subgroup `H`. It suffices to check
    // `zeta` only, since `(g * zeta)^n = zeta^n`, where `n` is the order of
    // `g`.
    let g = F::primitive_root_of_unity(degree_bits);
    ensure!(
        zeta.exp_power_of_2(degree_bits) != F::Extension::ONE,
        "Opening point is in the subgroup."
    );

    let openings = StarkOpeningSet::new(
        zeta,
        g,
        trace_commitment,
        &ctl_zs_commitment,
        &quotient_commitment,
        degree_bits,
    );

    challenger.observe_openings(&openings.to_fri_openings());

    let initial_merkle_trees = vec![trace_commitment, &ctl_zs_commitment, &quotient_commitment];

    // Make sure that we do not use Starky's lookups.
    assert!(!stark.requires_ctls());
    assert!(!stark.uses_lookups());
    let num_make_rows_public_data = public_sub_table_data.len();
    let opening_proof = timed!(
        timing,
        format!("{stark}: compute opening proofs").as_str(),
        PolynomialBatch::prove_openings(
            &stark.fri_instance(
                zeta,
                g,
                0,
                vec![],
                config,
                Some(&LookupConfig {
                    degree_bits,
                    num_zs: ctl_data.len() + num_make_rows_public_data
                })
            ),
            &initial_merkle_trees,
            challenger,
            &fri_params,
            timing,
        )
    );

    Ok(StarkProof {
        trace_cap: trace_commitment.merkle_tree.cap.clone(),
        ctl_zs_cap,
        quotient_polys_cap,
        openings,
        opening_proof,
    })
}

/// Given the traces generated from [`generate_traces`] along with their
/// commitments, prove a [`MozakStark`].
///
/// # Errors
/// Errors if proving fails.
#[allow(clippy::too_many_arguments)]
pub fn prove_with_commitments<F, C, const D: usize>(
    mozak_stark: &MozakStark<F, D>,
    config: &StarkConfig,
    public_inputs: &PublicInputs<F>,
    traces_poly_values: &TableKindArray<Vec<PolynomialValues<F>>>,
    trace_commitments: &TableKindArray<PolynomialBatch<F, C, D>>,
    ctl_data_per_table: &TableKindArray<CtlData<F>>,
    public_sub_data_per_table: &TableKindArray<CtlData<F>>,
    challenger: &mut Challenger<F, C::Hasher>,
    timing: &mut TimingTree,
) -> Result<TableKindArray<StarkProof<F, C, D>>>
where
    F: RichField + Extendable<D>,
    C: GenericConfig<D, F = F>, {
    let cpu_stark = [public_inputs.entry_point];
    let public_inputs = TableKindSetBuilder::<&[_]> {
        cpu_stark: &cpu_stark,
        ..Default::default()
    }
    .build();

    Ok(all_starks!(mozak_stark, |stark, kind| {
        prove_single_table(
            stark,
            config,
            &traces_poly_values[kind],
            &trace_commitments[kind],
            public_inputs[kind],
            &ctl_data_per_table[kind],
            &public_sub_data_per_table[kind],
            challenger,
            timing,
        )?
    }))
}

#[cfg(test)]
mod tests {

    use mozak_runner::instruction::{Args, Instruction, Op};
    use mozak_runner::poseidon2::MozakPoseidon2;
    use mozak_runner::util::execute_code;
    use plonky2::field::goldilocks_field::GoldilocksField;
    use plonky2::hash::poseidon2::Poseidon2Hash;
    use plonky2::plonk::config::{GenericHashOut, Hasher};

    use crate::stark::mozak_stark::MozakStark;
    use crate::test_utils::{create_poseidon2_test, Poseidon2Test, ProveAndVerify};

    #[test]
    fn prove_halt() {
        let (program, record) = execute_code([], &[], &[]);
        MozakStark::prove_and_verify(&program, &record).unwrap();
    }

    #[test]
    fn prove_lui() {
        let lui = Instruction {
            op: Op::ADD,
            args: Args {
                rd: 1,
                imm: 0x8000_0000,
                ..Args::default()
            },
        };
        let (program, record) = execute_code([lui], &[], &[]);
        assert_eq!(record.last_state.get_register_value(1), 0x8000_0000);
        MozakStark::prove_and_verify(&program, &record).unwrap();
    }

    #[test]
    fn prove_lui_2() {
        let (program, record) = execute_code(
            [Instruction {
                op: Op::ADD,
                args: Args {
                    rd: 1,
                    imm: 0xDEAD_BEEF,
                    ..Args::default()
                },
            }],
            &[],
            &[],
        );
        assert_eq!(record.last_state.get_register_value(1), 0xDEAD_BEEF,);
        MozakStark::prove_and_verify(&program, &record).unwrap();
    }

    #[test]
    fn prove_beq() {
        let (program, record) = execute_code(
            [Instruction {
                op: Op::BEQ,
                args: Args {
                    rs1: 0,
                    rs2: 1,
                    imm: 42, // branch target
                    ..Args::default()
                },
            }],
            &[],
            &[(1, 2)],
        );
        assert_eq!(record.last_state.get_pc(), 8);
        MozakStark::prove_and_verify(&program, &record).unwrap();
    }

    #[allow(unused)]
    fn test_poseidon2(test_data: &[Poseidon2Test]) {
        let (program, record) = create_poseidon2_test(test_data);
        for test_datum in test_data {
            let output: Vec<u8> = (0..32_u8)
                .map(|i| {
                    record
                        .last_state
                        .load_u8(test_datum.output_start_addr + u32::from(i))
                })
                .collect();
            let data_fields: Vec<GoldilocksField> = MozakPoseidon2::pack_padded_input(
                MozakPoseidon2::do_padding(test_datum.data.as_bytes()).as_slice(),
            );
            assert_eq!(
                output,
                Poseidon2Hash::hash_no_pad(&data_fields).to_bytes(),
                "Expected vm-computed output, does not equal to plonky2 version"
            );
        }
        MozakStark::prove_and_verify(&program, &record).unwrap();
    }

    #[test]
    #[cfg(feature = "enable_poseidon_starks")]
    fn prove_poseidon2() {
        test_poseidon2(&[Poseidon2Test {
            data: "💥 Mozak-VM Rocks With Poseidon2".to_string(),
            input_start_addr: 1024,
            output_start_addr: 2048,
        }]);
        test_poseidon2(&[Poseidon2Test {
            data: "😇 Mozak is knowledge arguments based technology".to_string(),
            input_start_addr: 1024,
            output_start_addr: 2048,
        }]);
        test_poseidon2(&[
            Poseidon2Test {
                data: "💥 Mozak-VM Rocks With Poseidon2".to_string(),
                input_start_addr: 512,
                output_start_addr: 1024,
            },
            Poseidon2Test {
                data: "😇 Mozak is knowledge arguments based technology".to_string(),
                input_start_addr: 1024 + 32,
                // make sure input and output do not overlap with
                // earlier call
                output_start_addr: 2048,
            },
        ]);
    }
}
