//! Check catalog publication outcomes before and after the first root replacement.
//!
//! A healthy sequence commits attempts 3 and 5, leaving attempt 4 aborted. Both
//! receipts must remain in success history, and the first data unit must retain
//! its bytes. The two committed graphs contain four and eight rows respectively.
//!
//! Record one healthy publication, then inject an error at every recorded effect
//! on a fresh fixture. Errors from the first root-replacement attempt onward must
//! be classified as uncertain. Selection and metadata repair must recover either
//! the old or new graph, with the corresponding row count and receipt history.
//! Separate invalid-transition cases must fail before any filesystem effect.
//!
//! These tests use production selection and repair helpers on constructed files.
//! They exercise injected failures, not process death or a complete database open.

use super::{Fixture, first_commit, genesis, graph, publish, second_admission, token};
use crate::CancellationToken;
use crate::effects::{Effect, Effects, Faults, LoadEffect};
use crate::storage::format::WalRecord;
use crate::storage::publication::{FailureStage, publish_snapshot};
use std::fs;
use std::sync::{Arc, Mutex};

#[test]
fn two_catalog_commits_preserve_successes_across_an_abort_gap() {
    let fixture = Fixture::new();
    let (first, unit) = first_commit(&fixture);
    let old_unit = fs::read(fixture.path(unit.object())).unwrap();
    let issued = second_admission(&fixture, first);
    let (second, _) = fixture.prepare(issued, Some(unit));
    publish(&fixture, issued, second);
    assert_eq!(fixture.selected(), (second, None));
    fixture.assert_graph(first, 4);
    fixture.assert_graph(second, 8);
    assert_eq!(fixture.find(second, 3), Some(1));
    assert_eq!(fixture.find(second, 5), Some(2));
    assert_eq!(fixture.find(second, 4), None);
    assert_eq!(fixture.find(second, 1), None);
    assert_eq!(fs::read(fixture.path(unit.object())).unwrap(), old_unit);
    assert_eq!(graph(second).transaction(), token(5));
    assert_eq!(graph(first).generation(), 1);
    assert_eq!(graph(second).generation(), 2);
}

#[test]
fn second_catalog_commit_effect_cuts_select_and_repair_an_honest_snapshot() {
    let baseline = Fixture::new();
    let (first, unit) = first_commit(&baseline);
    let issued = second_admission(&baseline, first);
    let (second, _) = baseline.prepare(issued, Some(unit));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let captured = trace.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            captured.lock().unwrap().push((index, effect))
        })),
        ..Faults::default()
    });
    if let Err(failure) = publish_snapshot(
        baseline.root(),
        issued,
        second,
        &CancellationToken::new(),
        &mut effects,
    ) {
        panic!("{}", failure.error)
    }
    let trace = trace.lock().unwrap();
    assert!(trace.len() < 64);
    // Even a reported failure of the first replacement is uncertain: callers
    // cannot assume the filesystem left the old name untouched.
    let point = trace
        .iter()
        .find(|(_, effect)| *effect == Effect::Load(LoadEffect::RenameRootA))
        .unwrap()
        .0;
    let mut old_seen = false;
    let mut new_seen = false;
    for cut in 0..effects.count() {
        let fixture = Fixture::new();
        let (first, unit) = first_commit(&fixture);
        let issued = second_admission(&fixture, first);
        let (second, _) = fixture.prepare(issued, Some(unit));
        let mut injected = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        let result = publish_snapshot(
            fixture.root(),
            issued,
            second,
            &CancellationToken::new(),
            &mut injected,
        );
        let failure = match result {
            Err(failure) => failure,
            Ok(()) => panic!("cut did not fail"),
        };
        assert_eq!(
            failure.stage == FailureStage::Uncertain,
            cut >= point,
            "cut {cut}"
        );
        let selected = fixture.heal_metadata(&mut Effects::default()).unwrap();
        assert!(selected == issued || selected == second);
        assert_eq!(fixture.find(selected, 3), Some(1));
        assert_eq!(fixture.find(selected, 4), None);
        if selected == second {
            new_seen = true;
            assert_eq!(fixture.find(selected, 5), Some(2));
            fixture.assert_graph(selected, 8);
        } else {
            old_seen = true;
            assert_eq!(fixture.find(selected, 5), None);
            fixture.assert_graph(selected, 4);
        }
    }
    // Require cuts on both sides of publication, not merely acceptance of
    // whichever outcome every case happened to produce.
    assert!(old_seen && new_seen);
}

#[test]
fn invalid_catalog_transitions_refuse_before_effects() {
    let fixture = Fixture::new();
    let (first, unit) = first_commit(&fixture);
    let issued = second_admission(&fixture, first);
    let (second, _) = fixture.prepare(issued, Some(unit));
    for (old, new) in [
        (first, second),
        (second, first),
        (second, second),
        (genesis(), second),
        (
            WalRecord {
                issued: 6,
                ..issued
            },
            second,
        ),
    ] {
        let mut effects = Effects::default();
        assert!(
            publish_snapshot(
                fixture.root(),
                old,
                new,
                &CancellationToken::new(),
                &mut effects
            )
            .is_err()
        );
        assert_eq!(effects.count(), 0);
    }
}
