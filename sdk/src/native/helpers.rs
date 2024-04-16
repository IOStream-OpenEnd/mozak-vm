use std::fs;
use std::path::PathBuf;

// This file contains code snippets used in native execution
use plonky2::field::goldilocks_field::GoldilocksField;
use plonky2::hash::poseidon2::Poseidon2Hash as Plonky2Poseidon2Hash;
use plonky2::plonk::config::{GenericHashOut, Hasher};
use poseidon2::mozak_poseidon2;

use crate::common::types::{Poseidon2Hash, ProgramIdentifier};

/// Represents a stack for call contexts during native execution.
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct IdentityStack(Vec<ProgramIdentifier>);

impl IdentityStack {
    pub fn add_identity(&mut self, id: ProgramIdentifier) { self.0.push(id); }

    pub fn top_identity(&self) -> ProgramIdentifier { self.0.last().copied().unwrap_or_default() }

    pub fn rm_identity(&mut self) { self.0.truncate(self.0.len().saturating_sub(1)); }
}

/// A bundle that declares the elf and system tape to be proven together.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ProofBundle {
    pub self_prog_id: String,
    pub elf_filepath: PathBuf,
    pub system_tape_filepath: PathBuf,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct GuestProgramTomlCfg {
    bin: Vec<Bin>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct Bin {
    name: String,
    path: String,
}

/// Manually add a `ProgramIdentifier` onto `IdentityStack`. Useful
/// when one want to escape automatic management of `IdentityStack`
/// via cross-program-calls sends (ideally temporarily).
/// CAUTION: Manual function for `IdentityStack`, misuse may lead
/// to system tape generation failure.
#[cfg(all(feature = "std", not(target_os = "mozakvm")))]
pub fn add_identity(id: crate::common::types::ProgramIdentifier) {
    unsafe {
        crate::common::system::SYSTEM_TAPE
            .call_tape
            .identity_stack
            .borrow_mut()
            .add_identity(id);
    }
}

/// Manually remove a `ProgramIdentifier` from `IdentityStack`.
/// Useful when one want to escape automatic management of `IdentityStack`
/// via cross-program-calls sends (ideally temporarily).
/// CAUTION: Manual function for `IdentityStack`, misuse may lead
/// to system tape generation failure.
#[cfg(all(feature = "std", not(target_os = "mozakvm")))]
pub fn rm_identity() {
    unsafe {
        crate::common::system::SYSTEM_TAPE
            .call_tape
            .identity_stack
            .borrow_mut()
            .rm_identity();
    }
}

/// Hashes the input slice to `Poseidon2Hash` after padding.
/// We use the well known "Bit padding scheme".
pub fn poseidon2_hash_with_pad(input: &[u8]) -> Poseidon2Hash {
    let data_fields: Vec<GoldilocksField> =
        mozak_poseidon2::pack_padded_input(mozak_poseidon2::do_padding(input).as_slice());
    Poseidon2Hash(
        Plonky2Poseidon2Hash::hash_no_pad(&data_fields)
            .to_bytes()
            .try_into()
            .expect("Output length does not match to DIGEST_BYTES"),
    )
}

/// Hashes the input slice to `Poseidon2Hash`, assuming
/// the slice length to be of multiple of `RATE`.
/// # Panics
/// If the slice length is not multiple of `RATE`.
/// This is intentional since zkvm's proof system
/// would fail otherwise.
#[allow(unused)]
pub fn poseidon2_hash_no_pad(input: &[u8]) -> Poseidon2Hash {
    assert!(input.len() % mozak_poseidon2::DATA_PADDING == 0);

    let data_fields: Vec<GoldilocksField> = mozak_poseidon2::pack_padded_input(input);

    Poseidon2Hash(
        Plonky2Poseidon2Hash::hash_no_pad(&data_fields)
            .to_bytes()
            .try_into()
            .expect("Output length does not match to DIGEST_BYTES"),
    )
}

/// Writes a byte slice to a given file
fn write_to_file(file_path: &str, content: &[u8]) {
    use std::io::Write;
    let path = std::path::Path::new(file_path);
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(content).unwrap();
}

/// Dumps a copy of `SYSTEM_TAPE` to disk, serialized
/// via `serde_json` as well as in rust debug file format
/// if opted for. Extension of `.tape.json` is used for serialized
/// formed of tape on disk, `.tape.debug` will be used for
/// debug tape on disk.
#[allow(dead_code)]
pub fn dump_system_tape(file_template: &str, is_debug_tape_required: bool) {
    let tape_clone = unsafe {
        crate::common::system::SYSTEM_TAPE.clone() // .clone() removes `Lazy{}`
    };

    if is_debug_tape_required {
        write_to_file(
            &(file_template.to_string() + ".tape_debug"),
            &format!("{tape_clone:#?}").into_bytes(),
        );
    }

    write_to_file(
        &(file_template.to_string() + ".tape.json"),
        &serde_json::to_string_pretty(&tape_clone)
            .unwrap()
            .into_bytes(),
    );
}

/// Gets the mozakvm binary name via reading the guest program's Cargo.toml,
/// and searching for a bin entry with path "..._mozak.rs".
pub(crate) fn get_mozak_binary_name() -> String {
    let toml_str = fs::read_to_string("Cargo.toml").expect(
        "Could not find the program's Cargo.toml. Are you running from within the
    project root?",
    );

    let toml: GuestProgramTomlCfg = toml::from_str(&toml_str).unwrap();

    // TODO(bing): Currently, this is used to derive the name of the mozakvm
    // binary based on the name declared alongside some path declared as
    // "..._mozak.rs" in the project's `Cargo.toml`. The purpose of that is so
    // we can dump the absolute path of the mozakvm ELF binary in our bundle
    // plan (JSON).
    //
    // It might be prudent to come up with a more robust solution than this.
    toml.bin
        .into_iter()
        .find(|b| b.path.contains("_mozak"))
        .expect(
            "Guest program does not have a mozakvm bin with path
 *_mozak.rs declared",
        )
        .name
}

/// This functions dumps 3 files of the currently running guest program:
///   1. the actual system tape (JSON),
///   2. the debug dump of the system tape,
///   3. the transaction bundle plan (JSON).
///
/// These are all dumped in a sub-directory named `out` in the project root. The
/// user must be cautious to not move the files, as the system tape and the
/// bundle plan are used by the CLI in proving and in transaction bundling.
pub fn dump_proving_files(file_template: &str, self_prog_id: ProgramIdentifier) {
    fs::create_dir_all("out").unwrap();
    let sys_tape_path = format!("out/{file_template}");
    dump_system_tape(&sys_tape_path, true);
    let bin_filename = format!("out/{file_template}.tape.json");

    let curr_dir = std::env::current_dir().unwrap();

    let bin_filepath_absolute = curr_dir.join(bin_filename);

    let native_exe = std::env::current_exe().unwrap();
    let mut components = native_exe.components();

    // Advance back by 3 iterations within the path components
    // to get to the target/ directory. In essence this gets rid of:
    // riscv32im-mozak-mozakvm-elf/release/<ELF_NAME>
    (0..3).for_each(|_| {
        components.next_back();
    });

    let elf_filepath = components.as_path().join(format!(
        "riscv32im-mozak-mozakvm-elf/release/{}",
        get_mozak_binary_name()
    ));

    let bundle = ProofBundle {
        self_prog_id: format!("{self_prog_id:?}"),
        elf_filepath,
        system_tape_filepath: bin_filepath_absolute,
    };
    println!("[BNDLDMP] Bundle dump: {bundle:?}");

    let bundle_filename = format!("out/{file_template}_bundle.json");
    let bundle_json = serde_json::to_string_pretty(&bundle).unwrap();
    write_to_file(&bundle_filename, bundle_json.as_bytes());
}
