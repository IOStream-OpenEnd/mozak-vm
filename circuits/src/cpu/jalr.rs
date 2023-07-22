use plonky2::field::packed::PackedField;
use plonky2::field::types::Field;
use starky::constraint_consumer::ConstraintConsumer;

use super::columns::{DST_VALUE, IMM_VALUE, NUM_CPU_COLS, OP1_VALUE, PC, S_JALR};

pub(crate) fn constraints<P: PackedField>(
    lv: &[P; NUM_CPU_COLS],
    nv: &[P; NUM_CPU_COLS],
    yield_constr: &mut ConstraintConsumer<P>,
) {
    let is_jalr = lv[S_JALR];
    let wrap_at = P::Scalar::from_noncanonical_u64(1 << 32);

    let return_address = lv[PC] + P::Scalar::from_noncanonical_u64(4);
    let wrapped_return_address = return_address - wrap_at;

    let destination = lv.dst_value;
    // enable-if JALR: aux.dst_val == jmp-inst-pc + 4, wrapped
    yield_constr.constraint(
        is_jalr * (destination - return_address) * (destination - wrapped_return_address),
    );

    let jump_target = lv.imm_value + lv[OP1_VALUE];
    let wrapped_jump_target = jump_target - wrap_at;
    let new_pc = nv[PC];

    yield_constr
        .constraint_transition(is_jalr * (new_pc - jump_target) * (new_pc - wrapped_jump_target));
}
#[cfg(test)]
mod tests {
    use mozak_vm::instruction::{Args, Instruction, Op};
    use mozak_vm::test_utils::{last_but_coda, reg, simple_test_code, u32_extra};
    use proptest::prelude::ProptestConfig;
    use proptest::proptest;

    use crate::test_utils::simple_proof_test;

    #[test]
    fn prove_jalr_goto_no_rs1() {
        let record = simple_test_code(
            &[Instruction {
                op: Op::JALR,
                args: Args {
                    rd: 0,
                    rs1: 0,
                    imm: 4,
                    ..Args::default()
                },
            }],
            &[],
            &[],
        );
        assert_eq!(record.last_state.get_pc(), 8);
        simple_proof_test(&record.executed).unwrap();
    }

    #[test]
    fn prove_jalr_goto_rs1_zero() {
        let record = simple_test_code(
            &[Instruction {
                op: Op::JALR,
                args: Args {
                    rd: 0,
                    rs1: 1,
                    imm: 4,
                    ..Args::default()
                },
            }],
            &[],
            &[(0x1, 0)],
        );
        assert_eq!(record.last_state.get_pc(), 8);
        simple_proof_test(&record.executed).unwrap();
    }
    #[test]
    fn prove_jalr_goto_imm_zero_rs1_not_zero() {
        let record = simple_test_code(
            &[Instruction {
                op: Op::JALR,
                args: Args {
                    rd: 0,
                    rs1: 1,
                    imm: 0,
                    ..Args::default()
                },
            }],
            &[],
            &[(0x1, 4)],
        );
        assert_eq!(record.last_state.get_pc(), 8);
        simple_proof_test(&record.executed).unwrap();
    }

    #[test]
    fn prove_jalr() {
        let record = simple_test_code(
            &[Instruction {
                op: Op::JALR,
                args: Args {
                    rd: 1,
                    rs1: 0,
                    imm: 4,
                    ..Args::default()
                },
            }],
            &[],
            &[(0x1, 0)],
        );
        assert_eq!(record.last_state.get_pc(), 8);
        simple_proof_test(&record.executed).unwrap();
    }

    #[test]
    fn prove_triple_jalr() {
        let record = simple_test_code(
            &[
                Instruction {
                    op: Op::JALR,
                    args: Args {
                        imm: 8, // goto to pc = 8
                        ..Args::default()
                    },
                },
                Instruction {
                    op: Op::JALR,
                    args: Args {
                        imm: 12, // goto to pc = 12
                        ..Args::default()
                    },
                },
                Instruction {
                    op: Op::JALR,
                    args: Args {
                        imm: 4, // goto to pc = 4
                        ..Args::default()
                    },
                },
            ],
            &[],
            &[],
        );
        assert_eq!(record.last_state.get_pc(), 16);
        simple_proof_test(&record.executed).unwrap();
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4))]
        #[test]
        fn jalr_jumps_past_an_instruction(rs1 in reg(), rs1_val in u32_extra(), rd in reg(), sentinel in u32_extra()) {
            let jump_target: u32 = 8;
            let imm = jump_target.wrapping_sub(rs1_val);
            let record = simple_test_code(
                &[Instruction {
                    op: Op::JALR,
                    args: Args {
                        rd,
                        rs1,
                        imm,
                        ..Args::default()
                    },
                },
                // We are jumping past this instruction, so it should not be executed.
                // So we should not overwrite register `rd` with `sentinel`.
                Instruction {
                    op: Op::ADD,
                    args: Args {
                        rd,
                        imm: sentinel,
                        ..Args::default()
                    },
                }],
                &[],
                &[(rs1, rs1_val)],
            );
            assert_eq!(record.executed.len(), 3);
            assert_eq!(last_but_coda(&record).get_register_value(rd), 4);
            simple_proof_test(&record.executed).unwrap();
        }
    }
}
