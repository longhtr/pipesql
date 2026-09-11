#!/usr/bin/env python3
"""Bounded root/fence representation challenge, NOT a filesystem simulator.

Roots abstract already validated, self-contained snapshots. None means a missing
or checksum-invalid current-format record, not a recognizable foreign/future
record (which must fail admission). Named replacements are atomic; root contents
and dependencies are durable before replacement. Successful directory barriers
persist replacements. A completed publication persists both roots before exposing
issuance/acknowledging data. These are premises, not results of this model.
"""

from dataclasses import dataclass, replace
from itertools import combinations, product


@dataclass(frozen=True)
class Root:
    revision: int
    issued: int
    successes: tuple[int, ...]

    @property
    def generation(self):
        return len(self.successes)

    def resolve(self, attempt):
        if not 1 <= attempt <= self.issued:
            return "not-found"
        if attempt in self.successes:
            return ("durable", self.successes.index(attempt) + 1)
        return "aborted"


@dataclass(frozen=True)
class Image:
    a: Root | None
    b: Root | None
    fence: Root | None


def successor(old, new):
    if new.revision != old.revision + 1:
        return False
    issuance = new.issued == old.issued + 1 and new.successes == old.successes
    commit = (
        new.issued == old.issued
        and old.issued > 0
        and old.issued not in old.successes
        and new.successes == old.successes + (old.issued,)
    )
    return issuance or commit


def recover(image):
    """Select without mutation. None is a refusal, never an abort answer."""
    roots = [root for root in (image.a, image.b) if root is not None]
    if not roots:
        return None
    selected = max(roots, key=lambda root: root.revision)
    if len(roots) == 2:
        older = min(roots, key=lambda root: root.revision)
        if older.revision == selected.revision:
            if older != selected or roots[0] != roots[1]:
                return None
        elif not successor(older, selected):
            return None
        # Both authority copies are known; an invalid pending fence cannot hide
        # a third, newer root. An intact fence must still be consistent.
        if image.fence is not None and not (
            image.fence == selected or successor(selected, image.fence)
        ):
            return None
    elif image.fence != selected:
        # A missing/invalid peer may have carried newer truth. Do not clear its
        # evidence or report abort from the older copy.
        return None
    return selected


def publication_cuts(old, new):
    assert successor(old, new)
    # Distinct crash images collapsed across write/readback/file-sync/rename/
    # directory-sync boundaries. No byte-level tear geometry is modeled.
    return (
        Image(old, old, old),
        Image(old, old, None),
        Image(old, old, new),
        Image(new, old, new),
        Image(new, new, new),
    )


def recovery_cuts(image, selected):
    # Repair/resync BOTH roots before replacing/resetting the fence. Only the
    # final completed image may admit a resolver or another writer.
    return (
        image,
        replace(image, a=selected),
        replace(image, a=selected, b=selected),
        Image(selected, selected, None),
        Image(selected, selected, selected),
    )


def preserved(known, selected):
    assert selected.issued >= known.issued
    for attempt in known.successes:
        assert selected.resolve(attempt) == known.resolve(attempt)


@dataclass(frozen=True)
class Durability:
    visible: Image
    durable: Image
    fence_dirty: bool = False


def effect(state, name, root):
    visible, durable, dirty = state.visible, state.durable, state.fence_dirty
    if name == "fence_partial":
        visible, dirty = replace(visible, fence=None), True
    elif name == "fence_write":
        visible, dirty = replace(visible, fence=root), True
    elif name == "fence_sync":
        durable, dirty = replace(durable, fence=visible.fence), False
    elif name == "replace_a":
        visible = replace(visible, a=root)
    elif name == "replace_b":
        visible = replace(visible, b=root)
    elif name == "directory_sync":
        durable = replace(durable, a=visible.a, b=visible.b)
    else:
        raise AssertionError(name)
    return Durability(visible, durable, dirty)


def trace(initial, root, operations):
    states = [initial]
    for name in operations:
        states.append(effect(states[-1], name, root))
    return states


PUBLICATION = (
    "fence_partial",
    "fence_write",
    "fence_sync",
    "replace_a",
    "directory_sync",
    "replace_b",
    "directory_sync",
)
REPAIR = (
    "replace_a",
    "replace_b",
    "directory_sync",
    "fence_partial",
    "fence_write",
    "fence_sync",
)


def crash_images(state):
    # Unflushed renames may independently persist old/new names. A dirty fence
    # can retain old bytes, new bytes, or a detected torn record. Root bytes and
    # referenced objects were already durably validated before naming them.
    choices = []
    for name in ("a", "b", "fence"):
        values = {getattr(state.durable, name), getattr(state.visible, name)}
        if name == "fence" and state.fence_dirty:
            values.add(None)
        choices.append(values)
    return {Image(*values) for values in product(*choices)}


def challenge_effects(edges, faults):
    publication_images = recovery_images = live_recoveries = 0
    crossed_images = crossed_refusals = crossed_repair_images = 0
    reached = set()
    for old, new in edges:
        clean = Image(old, old, old)
        states = trace(Durability(clean, clean), new, PUBLICATION)
        for phase, state in enumerate(states):
            # Failed effects include cuts before and after the operation; arbitrary
            # partial persistence is enumerated independently of visible success.
            for crashed in crash_images(state):
                selected = recover(crashed)
                assert selected in (old, new)
                preserved(old, selected)
                if phase == len(PUBLICATION):
                    assert selected == new
                publication_images += 1
                reached.add(phase)
                for damaged in faults:
                    crossed = replace(crashed, **{name: None for name in damaged})
                    admitted = recover(crossed)
                    crossed_images += 1
                    if admitted is None:
                        crossed_refusals += 1
                        continue
                    preserved(old, admitted)
                    if phase == len(PUBLICATION):
                        assert admitted == new
                    for repair in trace(Durability(crossed, crossed), admitted, REPAIR):
                        for interrupted in crash_images(repair):
                            assert recover(interrupted) == admitted
                            crossed_repair_images += 1
                repairs = trace(Durability(crashed, crashed), selected, REPAIR)
                for repair in repairs:
                    for interrupted in crash_images(repair):
                        assert recover(interrupted) == selected
                        recovery_images += 1
            # A failed operation can be followed by reopen WITHOUT process death.
            # Visible new roots cannot be treated as already durable names.
            selected = recover(state.visible)
            assert selected in (old, new)
            repairs = trace(state, selected, REPAIR)
            for repair in repairs:
                for interrupted in crash_images(repair):
                    observed = recover(interrupted)
                    assert observed in (old, new)
                    preserved(old, observed)
            for crashed in crash_images(repairs[-1]):
                assert recover(crashed) == selected
            live_recoveries += 1
    assert reached == set(range(len(PUBLICATION) + 1))

    empty, issued = Root(0, 0, ()), Root(1, 1, ())
    clean = Image(empty, empty, empty)
    states = trace(Durability(clean, clean), issued, PUBLICATION)
    before_a_barrier = states[4]
    assert before_a_barrier.visible == Image(issued, empty, issued)
    assert before_a_barrier.durable == Image(empty, empty, issued)

    # Executable negative controls: remove a protocol obligation, then require
    # that enumeration exposes the break instead of silently approving it.
    broken = trace(
        before_a_barrier,
        issued,
        tuple(name for name in REPAIR if name != "directory_sync"),
    )[-1]
    assert any(recover(image) != issued for image in crash_images(broken))
    unfenced = trace(
        Durability(clean, clean),
        issued,
        tuple(name for name in PUBLICATION if name != "fence_sync"),
    )
    assert any(
        recover(image) is None for state in unfenced for image in crash_images(state)
    )
    assert any(recover(image) == empty for image in crash_images(before_a_barrier))
    print(
        f"effect_crash_images={publication_images} interrupted_repair_images={recovery_images} live_reopens={live_recoveries} publication_phases={len(reached)}"
    )
    print(
        f"effect_corruption_observations={crossed_images} refusals={crossed_refusals} interrupted_corruption_repair_images={crossed_repair_images}"
    )
    print(
        "effect negative controls: skipped recovery directory sync; skipped fence sync; token exposure before issuance barrier"
    )


def recover_by_pair(image):
    # Alternative representation: no stored revision scalar. All currently
    # modeled publications advance issuance OR commit count, never neither.
    roots = [root for root in (image.a, image.b) if root is not None]
    if not roots:
        return None
    selected = max(roots, key=lambda root: (root.issued, root.generation))

    def next_pair(old, new):
        return (new.issued == old.issued + 1 and new.successes == old.successes) or (
            new.issued == old.issued
            and old.issued > 0
            and old.issued not in old.successes
            and new.successes == old.successes + (old.issued,)
        )

    if len(roots) == 1:
        return selected if image.fence == selected else None
    older = min(roots, key=lambda root: (root.issued, root.generation))
    if roots[0] != roots[1] and not next_pair(older, selected):
        return None
    if (
        image.fence is not None
        and image.fence != selected
        and not next_pair(selected, image.fence)
    ):
        return None
    return selected


def challenge_derived_revision():
    roots = [None]
    for issued in range(4):
        for count in range(issued + 1):
            for successes in combinations(range(1, issued + 1), count):
                roots.append(Root(issued + count, issued, successes))
    observations = 0
    for values in product(roots, repeat=3):
        image = Image(*values)
        assert recover_by_pair(image) == recover(image)
        observations += 1
    # With a stored u64 revision, the final otherwise valid attempt is refused
    # to leave room for commit. A pair needs only issuance and commit capacities;
    # its abstract rank can exceed u64 without storing or narrowing that rank.
    maximum = (1 << 64) - 1
    old = Root(maximum - 1, maximum - 1, ())
    issued = Root(maximum, maximum, ())
    committed = Root(maximum + 1, maximum, (maximum,))
    assert not admission(old, maximum, maximum, 1)
    for image in publication_cuts(issued, committed):
        selected = recover_by_pair(image)
        assert selected in (issued, committed)
        assert selected.resolve(maximum - 1) == "aborted"
    assert recover_by_pair(Image(committed, committed, committed)).resolve(maximum) == (
        "durable",
        1,
    )
    print(
        f"derived_revision_observations={observations}; final-u64-attempt boundary preserved without stored rank"
    )


def objects(root):
    # A full-replacement data snapshot and a copied complete outcome index.
    # Issuance changes neither object. Bytes/checksums are not modeled here.
    if not root.successes:
        return frozenset()
    return frozenset((("data", root.generation), ("outcomes", root.successes)))


def challenge_reclamation(edges):
    observations = 0
    for old, new in edges:
        available = objects(old) | objects(new)  # New dependencies synced first.
        clean = Image(old, old, old)
        published = trace(Durability(clean, clean), new, PUBLICATION)[-1]
        assert published.durable.a == published.durable.b == new
        for pins in ((), (old,)):
            protected = objects(new)
            for pinned in pins:
                protected |= objects(pinned)
            retired = tuple(available - protected)
            # Unlink interruption can leave any subset of unreferenced files.
            # Actual admission/pin copying must be coherent and exclude a new pin
            # on a retired snapshot; no engine lock may cover the unlink I/O.
            for count in range(len(retired) + 1):
                for removed in combinations(retired, count):
                    remaining = available - set(removed)
                    for crashed in crash_images(published):
                        selected = recover(crashed)
                        assert objects(selected) <= remaining
                        for pinned in pins:
                            assert objects(pinned) <= remaining
                        for attempt in range(1, old.issued + 1):
                            if attempt in old.successes:
                                assert selected.resolve(attempt) == old.resolve(attempt)
                        observations += 1

    old, new = Root(3, 2, (1,)), Root(4, 2, (1, 2))
    clean = Image(old, old, old)
    premature = trace(Durability(clean, clean), new, PUBLICATION)[4]
    remaining = objects(new)  # Incorrectly unlink old graph after visible A.
    assert any(
        not objects(recover(image)) <= remaining for image in crash_images(premature)
    )
    # Even after durable publication, an old live snapshot pins its objects.
    assert not objects(old) <= remaining
    # After releasing that pin, data 1/history-copy 1 may disappear, but the new
    # complete index must still answer the original successful generation.
    assert new.resolve(1) == ("durable", 1)
    assert ("data", 1) not in remaining
    print(
        f"reclamation_observations={observations}; negative controls: premature unlink and ignored reader pin"
    )


def admission(root, attempt_limit, revision_limit, index_capacity):
    # One serialized writer reserves both issuance and possible commit revisions,
    # plus one successful-index entry, before any issuance effects. Abort can leave
    # that second revision unused; it does not recycle an issued attempt.
    return (
        root.issued + 1 <= attempt_limit
        and root.revision + 2 <= revision_limit
        and root.generation + 1 <= index_capacity
    )


def challenge_admission():
    root = Root(3, 2, (1,))
    for attempt_limit, revision_limit, index_capacity in product(
        (2, 3), (4, 5), (1, 2)
    ):
        before = root
        allowed = admission(root, attempt_limit, revision_limit, index_capacity)
        assert allowed == ((attempt_limit, revision_limit, index_capacity) == (3, 5, 2))
        assert root == before
    # Reserving only issuance would admit a load that cannot possibly commit.
    assert root.revision + 1 <= 4 and not admission(root, 3, 4, 2)
    print(
        "admission_boundaries=8; one-revision-only reservation counterexample retained; toy limits are not production quotas"
    )


def main():
    if not __debug__:
        raise SystemExit("model requires Python assertions")
    edges = []
    for issued in range(4):
        for count in range(issued + 1):
            for successes in combinations(range(1, issued + 1), count):
                old = Root(issued + count, issued, successes)
                if issued < 3:
                    # Seed is quiescent; any preceding unused ID is terminal.
                    edges.append((old, Root(old.revision + 1, issued + 1, successes)))
                if issued > 0 and issued not in successes:
                    # This seed has a LIVE active attempt == issued. Recovery
                    # would clear that authority; it could not then commit it.
                    edges.append(
                        (old, Root(old.revision + 1, issued, successes + (issued,)))
                    )

    faults = (
        (),
        ("a",),
        ("b",),
        ("fence",),
        ("a", "fence"),
        ("b", "fence"),
        ("a", "b"),
    )
    observations = refusals = recovery_observations = 0
    for old, new in edges:
        for cut, image in enumerate(publication_cuts(old, new)):
            assert recover(image) in (old, new)
            for damaged in faults:
                crossed = replace(image, **{name: None for name in damaged})
                selected = recover(crossed)
                observations += 1
                if selected is None:
                    refusals += 1
                    continue
                assert selected in (old, new)
                preserved(old, selected)
                if cut == 4:  # Issuance/data acknowledgment is now permitted.
                    assert selected == new
                # An issuance step cannot retroactively commit an old abort.
                for attempt in range(1, old.issued + 1):
                    if new.generation == old.generation:
                        assert selected.resolve(attempt) == old.resolve(attempt)
                for interrupted in recovery_cuts(crossed, selected):
                    # Continuing I/O interruption, then a healed retry. Already
                    # observed damage is retained, not magically undone.
                    retry = recover(interrupted)
                    assert retry == selected
                    recovery_observations += 1
                settled = Image(selected, selected, selected)
                assert recover(settled) == selected
                # Recovery has no live attempt. A later writer must issue the
                # NEXT ID before it can commit, never resume a settled gap.
                next_issued = Root(
                    selected.revision + 1, selected.issued + 1, selected.successes
                )
                next_commit = Root(
                    next_issued.revision + 1,
                    next_issued.issued,
                    next_issued.successes + (next_issued.issued,),
                )
                for later in publication_cuts(next_issued, next_commit):
                    latest = recover(later)
                    for attempt in range(1, selected.issued + 1):
                        assert latest.resolve(attempt) == selected.resolve(attempt)

    empty = Root(0, 0, ())
    issued = Root(1, 1, ())
    committed = Root(2, 1, (1,))
    issued_again = Root(3, 2, (1,))

    # Old data must not depend on the overwritten sole pending WAL/fence.
    pending = Image(committed, committed, issued_again)
    assert recover(pending) == committed
    sole_wal_graph_admits = pending.fence == committed
    assert not sole_wal_graph_admits

    # Catalog generation alone cannot order these healthy root publications.
    split = Image(issued, empty, issued)
    assert split.a.generation == split.b.generation and split.a != split.b
    assert recover(split) == issued

    # Wrong repair ordering destroys the last proof about an invalid peer.
    damaged = Image(issued, None, issued)
    assert recover(damaged) == issued
    assert recover(replace(damaged, fence=None)) is None

    # Live reopen after failed A directory sync: A is visibly new but BOTH
    # durable names may remain old. A repair that skips its barrier can expose
    # Aborted(1), then crash/next issuance can reuse 1. Not a filesystem run.
    visible = Image(issued, empty, issued)
    durable = Image(empty, empty, issued)
    selected = recover(visible)
    assert selected.resolve(1) == "aborted"
    after_skipped_barrier = recover(durable)
    assert after_skipped_barrier.resolve(1) == "not-found"
    reused = Root(
        after_skipped_barrier.revision + 1, after_skipped_barrier.issued + 1, ()
    )
    later = Root(reused.revision + 1, reused.issued, (reused.issued,))
    assert later.resolve(1) == ("durable", 1)

    challenge_effects(edges, faults)
    challenge_reclamation(edges)
    challenge_admission()
    challenge_derived_revision()
    print(
        f"edges={len(edges)} crash_corruption_observations={observations} refusals={refusals} healed_recovery_observations={recovery_observations}"
    )
    print(
        "counterexamples: sole-WAL graph dependency; generation-only ordering; fence-first repair; skipped live-recovery barrier"
    )
    print(
        "scope: abstract validated facts and declared persistence premises; no production crash, corruption, or resource qualification"
    )


if __name__ == "__main__":
    main()
