//! Name the exact engine tests selected for memory and concurrency instrumentation.
//!
//! These lists group CLI and library paths by the boundary the diagnostic exercises,
//! including native argument handling, import, query, export and shared ownership.
//! The sanitizer runner requires discovery and completion for every listed name,
//! so moving or renaming a test cannot silently shrink coverage. Adding a name
//! expands this bounded selection; it does not imply coverage of unlisted paths
//! or instrumentation of every native runtime library.

pub(super) const MEMORY_CLI_TESTS: &[&str] = &[
    "cli::arguments::tests::capture_matches_the_native_process_arguments",
    "cli::arguments::tests::native_copy_checks_extents_and_preserves_empty_raw_arguments",
    "cli::arguments::tests::stream_preserves_empty_and_non_utf8_arguments",
    "cli::arguments::tests::stream_bounds_even_the_ignored_program_name",
    "cli::arguments::tests::stream_checks_count_lengths_terminators_and_io",
    "cli::stdio::tests::duplicate_owns_a_separate_close_on_exec_descriptor",
    "cli::stdio::tests::owned_buffer_flushes_lines_capacity_and_tail",
    "cli::stdio::tests::finite_writer_preserves_short_writes_and_refuses_no_progress",
];

pub(super) const MEMORY_LIBRARY_TESTS: &[&str] = &[
    "csv::tests::every_split_preserves_unicode_quotes_endings_and_nulls",
    "csv::tests::limits_accept_exact_boundaries_and_locate_first_excess",
    "csv::tests::largest_fields_cross_read_buffers_without_growth",
    "csv::tests::maximum_column_and_batch_counts_preserve_declaration_order",
    "csv::import::tests::successful_import_reports_issuance_before_reading_and_preserves_old_snapshot",
    "csv::import::tests::late_bad_row_aborts_written_prefix_and_allows_retry",
    "csv::import::tests::late_reader_failure_and_cancellation_discard_private_units",
    "csv::import::tests::wide_text_batch_splits_at_native_column_limit",
    "csv::import::tests::conversion_keeps_repeated_types_and_partial_validity_bytes_separate",
    "csv::import::tests::failures::import_effect_failures_recover_only_complete_old_or_new_tables",
    "csv::import::tests::failures::reader_panic_requires_reopen_and_discards_written_prefix",
    "csv::import::tests::failures::cleanup_failure_retains_input_error_and_reopen_removes_prefix",
    "parquet::decoder::tests::short_reads_multiple_pages_and_batch_boundaries_preserve_all_rows",
    "parquet::decoder::tests::read_seek_and_completion_failures_stop_without_exposing_partial_batches",
    "parquet::import::tests::complete_import_reopens_with_all_values_and_resolved_receipt",
    "parquet::import::tests::final_source_failure_aborts_all_private_batches_and_allows_retry",
    "parquet::import::tests::failures::effect_refusals_recover_complete_old_or_new_tables",
    "parquet::import::tests::failures::cleanup_failure_retains_source_failure_and_recovers_without_a_prefix",
    "parquet::import::tests::failures::final_cancellation_aborts_and_reader_panic_requires_reopen",
    "parquet::import::tests::interruption::process_interruption_resolves_empty_or_complete_import",
    "jsonl::tests::complete_rows_and_exact_limits_release_ownership",
    "jsonl::tests::typed_cursor_exports_nullable_columns_and_reordered_schema",
    "jsonl::tests::short_writes_and_output_failures_preserve_terminal_outcome",
    "jsonl::tests::cancellation_and_late_query_failure_release_and_allow_retry",
    "jsonl::tests::panicking_writer_drops_execution_and_buffer_reservations",
    "parquet::export::tests::complete_output_limits_and_cursor_independent_grouping",
    "parquet::export::tests::complete_scalar_export_matches_external_reader_fixture",
    "parquet::export::tests::writer_failures_cancellation_and_panic_release_owners",
    "parquet::export::tests::late_query_error_preserves_span_and_admission_refusal_writes_nothing",
];

pub(super) const MEMORY_CATALOG_TESTS: &[&str] = &[
    "snapshots::overlapping_reports_preserve_pins_through_publication_and_cancellation",
    "joins::typed_join_matches_an_independent_row_oracle_through_spill",
    "multiset::public_multiset_preserves_typed_classes_original_bits_and_snapshots",
    "window_sum::running_sum_keeps_exact_peer_totals_and_demands_only_visible_overflow",
];
pub(super) const CONCURRENT_LIBRARY_TESTS: &[&str] = &[
    "resources::tests::temporary_authority_preserves_shared_capacity_and_overflow",
    "resources::tests::temporary_worker_failure_controls_terminate",
    "query::binding::tests::exact_prepared_admission_and_concurrent_owners",
    "query::binding::tests::prepare_worker_failure_controls_terminate",
    "execution::aggregation::tests::concurrent_aggregate_readers_reconcile_one_memory_account",
    "execution::aggregation::tests::aggregate_worker_failure_controls_terminate",
    "execution::scan::legacy::tests::cancellation_at_every_effect_and_from_real_thread",
    "execution::scan::legacy::tests::scan_thread_failure_controls_terminate",
    "storage::snapshot::tests::snapshots::database_snapshot_survives_real_publication_and_abort_gap",
    "storage::snapshot::tests::snapshots::registry_completion_waits_and_poisoning_preserves_old_views",
    "storage::reclaim::tests::scratch::scratch_bootstrap_real_thread_excludes_only_another_bootstrap",
    "storage::reclaim::tests::scratch::scratch_bootstrap_failure_controls_terminate",
];

pub(super) const CONCURRENT_CATALOG_TESTS: &[&str] = &[
    "snapshots::concurrent_readers_keep_generations_through_reclamation_and_early_drop",
    "snapshots::overlapping_reports_preserve_pins_through_publication_and_cancellation",
    "snapshots::overlapping_report_control_detects_unlinked_pinned_catalog",
];
