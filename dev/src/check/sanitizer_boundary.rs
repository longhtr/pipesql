//! Compare native boundary tests across compiler and instrumentation configurations.
//!
//! The selected mutex or pathname cases run under the pinned compiler, plain nightly
//! and the requested sanitizer. Test discovery and completion must contain every
//! required name before results are accepted. Cargo identifies each test executable;
//! optional rebuilt-standard-library checks establish which runtime it uses.
//! Even a successful test exit is rejected if its diagnostic output reports a
//! sanitizer fault. Broader engine selections belong to the parent campaign.

use super::*;

pub(super) fn run(
    run: &mut Run,
    nightly: &str,
    host: &str,
    case: &str,
    standard: Option<&standard::StandardLibrary>,
) -> Result<()> {
    let mut required = vec![
        "mutex::tests::stationary_owner_moves_and_nonblocking_reentry_refuses",
        "mutex::tests::native_mutex_serializes_real_threads_and_drops_value_once",
        "mutex::tests::panic_poisoning_refuses_protected_state",
        "mutex::tests::forgotten_guard_keeps_native_storage_alive",
    ];
    if case == "pathname" || case == "memory" {
        let mutex = required;
        required = vec![
            "tests::decoder_bounds_and_mutations",
            "tests::decoder_reviewed_abi",
            "tests::native_canonical_names_match_reference_on_joined_workers",
            "tests::native_directory_names_refusal_and_independent_cursors",
            "tests::native_directory_cursors_are_independent_on_real_threads",
            "tests::native_path_metadata_matches_independent_std_and_opened_files",
            "tests::native_path_mutations_and_canonicalization_match_std",
            "tests::native_directory_open_errors_and_long_path",
            "tests::directory_total_bound_is_independent_of_per_call_work",
        ];
        if cfg!(target_os = "macos") {
            required.extend([
                "tests::decoder_flags_offsets_and_native_error",
                "tests::native_deep_absolute_links_preserve_the_33_link_boundary",
                "syscall::path::tests::native_budget_refuses_the_next_call_and_stays_exhausted",
                "syscall::path::tests::root_resolution_obeys_admission_before_native_effects",
                "syscall::path::tests::invalid_input_and_slot_growth_do_not_publish",
                "syscall::path::name_record::tests::offset_length_and_termination_are_independent",
                "syscall::path::name_record::tests::maximum_name_and_non_utf8_are_borrowed_without_allocation",
            ]);
        } else {
            required.extend([
                "tests::canonicalization_preserves_expanded_suffixes_with_a_short_final_name",
                "tests::canonicalization_preserves_native_directory_permission_errors",
                "tests::canonicalization_preserves_errors_at_the_native_name_ceiling",
                "syscall::linux_path::tests::native_admission_precedes_entry_and_preserves_native_errors",
                "syscall::linux_path::tests::suffix_refusal_preserves_pending_bytes_and_typed_cause",
            ]);
        }
        if case == "memory" {
            required.extend(mutex);
        }
    }
    let mut suites = vec![(
        "filesystem",
        vec!["-p", "pipesql-filesystem", "--lib"],
        "pipesql_filesystem",
        required,
    )];
    if case == "memory" {
        suites.extend([
            (
                "cli",
                vec!["-p", "pipesql", "--bin", "pipesql"],
                "pipesql",
                cases::MEMORY_CLI_TESTS.to_vec(),
            ),
            (
                "library",
                vec!["-p", "pipesql", "--lib"],
                "pipesql",
                cases::MEMORY_LIBRARY_TESTS.to_vec(),
            ),
            (
                "public",
                vec!["-p", "pipesql", "--test", "public"],
                "public",
                cases::MEMORY_CATALOG_TESTS.to_vec(),
            ),
        ]);
    }
    if case == "concurrency" {
        suites.extend([
            (
                "library",
                vec!["-p", "pipesql", "--lib"],
                "pipesql",
                cases::CONCURRENT_LIBRARY_TESTS.to_vec(),
            ),
            (
                "public",
                vec!["-p", "pipesql", "--test", "public"],
                "public",
                cases::CONCURRENT_CATALOG_TESTS.to_vec(),
            ),
        ]);
    }
    let thread = case == "concurrency";
    let kind = if thread { "thread" } else { "address" };
    let toolchain = fs::read_to_string(run.root.join("rust-toolchain.toml"))?;
    let pinned = toolchain
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("channel = \"")
                .and_then(|v| v.strip_suffix('"'))
        })
        .ok_or("pinned compiler selector is missing")?;
    let mut compiler = Command::new("rustc");
    compiler.args([&format!("+{pinned}"), "-vV"]);
    environment(&mut compiler);
    let output = run.command(&mut compiler, None, Duration::from_secs(30))?;
    output.require_success()?;
    if !std::str::from_utf8(&output.stdout.bytes)?
        .lines()
        .any(|line| line == format!("host: {host}"))
    {
        return Err("stock and nightly compiler hosts differ".into());
    }
    libraries(run, pinned, false, thread)?;
    for (label, selector, instrumented) in [
        ("stock", pinned, false),
        ("nightly", nightly, false),
        (kind, nightly, true),
    ] {
        let target = run.directory.join(label);
        let temporary = run.directory.join(format!("{label}-temporary"));
        fs::create_dir(&temporary)?;
        let configure = |command: &mut Command| {
            environment(command);
            command
                .env("CARGO_TARGET_DIR", &target)
                .env(
                    "RUSTFLAGS",
                    if instrumented {
                        if thread {
                            "-Dwarnings -Zsanitizer=thread"
                        } else {
                            "-Dwarnings -Zsanitizer=address"
                        }
                    } else {
                        "-Dwarnings"
                    },
                )
                .env("TMPDIR", &temporary)
                .env("TMP", &temporary)
                .env("TEMP", &temporary);
        };
        for (suite, arguments, artifact, required) in &suites {
            let prefix = format!("{label}-{suite}");
            let mut build = Command::new("cargo");
            build.args([
                &format!("+{selector}"),
                "test",
                "--release",
                "--offline",
                "--locked",
                "-j",
                "1",
                "--target",
                host,
                "--no-run",
                "--message-format=json",
            ]);
            build.args(arguments);
            configure(&mut build);
            if label != "stock"
                && let Some(standard) = standard
            {
                standard.configure(&mut build)?;
            }
            let output = run.command(&mut build, None, Duration::from_secs(600))?;
            output.require_success()?;
            if label != "stock" && standard.is_some() {
                let library = standard::rebuilt(&output, &target)?;
                fs::write(
                    run.directory
                        .join(format!("{prefix}-standard-library.sha256")),
                    workspace::hash(&library)?,
                )?;
            }
            let binary = workspace::cargo_artifact(
                &output,
                artifact,
                workspace::Artifact::Executable { test: true },
            )?
            .canonicalize()?;
            if !binary.starts_with(target.canonicalize()?) {
                return Err("sanitizer test artifact is outside the owned target".into());
            }
            let binary = &binary;
            fs::write(
                run.directory.join(format!("{prefix}.sha256")),
                workspace::hash(binary)?,
            )?;
            if instrumented {
                let mut command = Command::new(if cfg!(target_os = "macos") {
                    "otool"
                } else {
                    "ldd"
                });
                if cfg!(target_os = "macos") {
                    command.arg("-L");
                }
                command.arg(binary);
                configure(&mut command);
                run.command(&mut command, None, Duration::from_secs(30))?
                    .require_success()?;
            }
            for completed in [false, true] {
                let mut command = Command::new(binary);
                command.args(required).arg("--exact").arg(if completed {
                    "--test-threads=1"
                } else {
                    "--list"
                });
                configure(&mut command);
                let output = run.command(&mut command, None, Duration::from_secs(180))?;
                output.require_success()?;
                selection(
                    std::str::from_utf8(&output.stdout.bytes)?,
                    required,
                    completed,
                )?;
                if std::str::from_utf8(&output.stderr.bytes)?.contains("Sanitizer") {
                    return Err("sanitizer reported a fault during successful tests".into());
                }
            }
            println!(
                "sanitizer {case} {prefix}: all {} tests discovered and passed",
                required.len()
            );
        }
    }
    Ok(())
}
