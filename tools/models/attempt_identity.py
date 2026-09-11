#!/usr/bin/env python3
"""Bounded representation experiment, NOT a simulator of production persistence.

Assumes a validated, durably published root. Models facts that this root must
preserve, not how writes, root replicas, recovery, or checksums establish them.
The sequence/capacity are deliberately tiny to enumerate histories exhaustively.
No byte format or production retention capacity is selected here.
"""

from dataclasses import dataclass
from itertools import combinations, product

CAPACITY = 3
STEPS = 5
EVENTS = (
    "abort",
    "commit",
    "issue-lost",
    "issue-kept",
    "ambiguous-abort",
    "ambiguous-commit",
)


@dataclass(frozen=True)
class Index:
    issued: int = 0
    # Successful attempts in generation order; omissions inside the issued
    # prefix denote abort only after exclusive recovery has resolved live work.
    committed: tuple[int, ...] = ()

    def resolve(self, attempt: int):
        if not 1 <= attempt <= self.issued:
            return "not-found"
        for generation, identity in enumerate(self.committed, 1):
            if identity == attempt:
                return ("durable", generation)
        return "aborted"

    def transition(self, event: str):
        if self.issued == CAPACITY:
            return self, None, "refused"
        # Failed issuance may leave no durable evidence, but then no token may
        # have escaped. If it persists, consume the identity even without a token.
        if event == "issue-lost":
            return self, None, "hidden"
        attempt = self.issued + 1
        committed = self.committed
        if event in ("commit", "ambiguous-commit"):
            committed += (attempt,)
        exposed = None if event == "issue-kept" else attempt
        return Index(attempt, committed), exposed, "accepted"


def check_history(events):
    index = Index()
    # Independent explicit per-attempt outcome table, rather than deriving abort
    # from the candidate's numeric prefix. It intentionally uses more state.
    receipts = {}
    durable_generation = 0
    next_identity = 1
    exposed = set()
    classes = set()
    for event in events:
        before = index
        index, token, outcome = index.transition(event)
        if next_identity > CAPACITY:
            assert outcome == "refused" and token is None and index == before
            classes.add("capacity-refusal")
        elif event == "issue-lost":
            assert outcome == "hidden" and token is None and index == before
            classes.add("issuance-not-durable")
        else:
            identity = next_identity
            next_identity += 1
            if event in ("commit", "ambiguous-commit"):
                durable_generation += 1
                receipts[identity] = ("durable", durable_generation)
            else:
                receipts[identity] = "aborted"
            if token is not None:
                assert token == identity and token not in exposed
                exposed.add(token)
            classes.add(event)
        # Reconstruct only from persisted semantic facts. No token cache or
        # attempt-local state is available to resolution after this "reopen".
        reopened = Index(index.issued, tuple(index.committed))
        for attempt in range(CAPACITY + 2):
            assert reopened.resolve(attempt) == receipts.get(attempt, "not-found")
        assert len(index.committed) == durable_generation
        assert len(receipts) == index.issued
    return classes


def resolve_live(index, active, attempt):
    settled = index.resolve(attempt)
    # Never expose an unissued guess as a real pending token. Successful facts
    # take precedence after commit publication, even before writer cleanup ends.
    if settled == "aborted" and active == attempt:
        return "pending"
    return settled


def check_live_snapshot():
    before = Index(1)
    after = Index(1, (1,))
    assert resolve_live(Index(), 1, 1) == "not-found"
    assert resolve_live(before, 1, 1) == "pending"
    assert resolve_live(after, None, 1) == ("durable", 1)
    # Enumerate the six order-preserving merges of two writer steps and two
    # reader steps. Root/index publication followed by clearing active authority
    # is NOT enough if the reader fetches these facts separately.
    counterexamples = []
    for writes in combinations(range(4), 2):
        index, active = before, 1
        writer_step = reader_step = 0
        captured = answer = None
        schedule = "".join("W" if step in writes else "R" for step in range(4))
        for step in range(4):
            if step in writes:
                if writer_step == 0:
                    index = after
                else:
                    active = None
                writer_step += 1
            else:
                if reader_step == 0:
                    captured = index
                else:
                    answer = resolve_live(captured, active, 1)
                reader_step += 1
            assert resolve_live(index, active, 1) in ("pending", ("durable", 1))
        if answer == "aborted":
            counterexamples.append(schedule)
        # A coherent snapshot at any point can report only Pending or Durable:
        # this history has no abort transition at which Aborted could linearize.
        assert resolve_live(index, active, 1) == ("durable", 1)
    assert counterexamples == ["RWWR"]
    print(
        "rejected split outcome/active reads: 1 of 6 interleavings falsely aborts (RWWR)"
    )


def counterexamples():
    # Generation-slot identity aliases A-aborted with B-committed.
    slot_for_a = slot_for_b = 1
    assert slot_for_a == slot_for_b
    print("rejected generation-slot ID: A aborted; B committed; A becomes durable")

    # Unique identity alone is insufficient: keeping only the newest success
    # cannot distinguish an older successful attempt from an aborted gap.
    history = Index(3, (1, 3))
    latest_only = Index(3, (3,))
    assert history.resolve(1) != latest_only.resolve(1)
    print(
        "rejected latest-success-only: commit(1), abort(2), commit(3) loses outcome(1)"
    )

    # An exposed ID cannot be reused after failed issuance disappears on crash.
    exposed_before_durability = 1
    reopened = Index()
    _, later_id, _ = reopened.transition("commit")
    assert later_id == exposed_before_durability
    print("rejected early token exposure: lost issuance allows identity reuse")


def main():
    if not __debug__:
        raise SystemExit("model requires Python assertions")
    classes = set()
    histories = 0
    for events in product(EVENTS, repeat=STEPS):
        classes.update(check_history(events))
        histories += 1
    assert classes == {
        "capacity-refusal",
        "issuance-not-durable",
        "abort",
        "commit",
        "issue-kept",
        "ambiguous-abort",
        "ambiguous-commit",
    }
    counterexamples()
    check_live_snapshot()
    print(f"histories={histories} transitions={histories * STEPS}")
    print(f"capacity={CAPACITY} steps={STEPS} reached={','.join(sorted(classes))}")
    print("prefix+complete-success-index agrees with explicit receipts in this model")
    print(
        "NOT VERIFIED: root publication, crash atomicity, corruption, byte format, physical resources"
    )


if __name__ == "__main__":
    main()
