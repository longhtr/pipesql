//! Exhaust small transaction histories and publication/repair states.
//!
//! This model assumes atomic root replacement, durable referenced objects, and
//! directory synchronization that persists names. It separates visible changes
//! from durable changes, including repair after failed I/O without a restart.
//! It executes no database code and establishes no native storage guarantee.

use crate::Result;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Root {
    issued: u64,
    successes: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Receipt {
    Missing,
    Pending,
    Aborted,
    Durable(usize),
}

impl Root {
    fn empty() -> Self {
        Self {
            issued: 0,
            successes: Vec::new(),
        }
    }
    fn issue(&self) -> Self {
        Self {
            issued: self.issued.checked_add(1).unwrap(),
            successes: self.successes.clone(),
        }
    }
    fn commit(&self) -> Self {
        assert!(self.issued > 0 && !self.successes.contains(&self.issued));
        let mut result = self.clone();
        result.successes.push(self.issued);
        result
    }
    fn resolve(&self, attempt: u64) -> Receipt {
        if attempt == 0 || attempt > self.issued {
            return Receipt::Missing;
        }
        self.successes
            .iter()
            .position(|number| *number == attempt)
            .map_or(Receipt::Aborted, |index| Receipt::Durable(index + 1))
    }
    fn live(&self, active: Option<u64>, attempt: u64) -> Receipt {
        match self.resolve(attempt) {
            Receipt::Aborted if active == Some(attempt) => Receipt::Pending,
            other => other,
        }
    }
    fn follows(&self, old: &Self) -> bool {
        (old.issued.checked_add(1) == Some(self.issued) && self.successes == old.successes)
            || (self.issued == old.issued
                && old.issued != 0
                && !old.successes.contains(&old.issued)
                && self.successes.len() == old.successes.len() + 1
                && self.successes.starts_with(&old.successes)
                && self.successes.last() == Some(&old.issued))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Image {
    a: Option<Root>,
    b: Option<Root>,
    fence: Option<Root>,
}
impl Image {
    fn settled(root: &Root) -> Self {
        Self {
            a: Some(root.clone()),
            b: Some(root.clone()),
            fence: Some(root.clone()),
        }
    }
    fn recover(&self) -> Option<Root> {
        let selected = match (&self.a, &self.b) {
            (Some(a), Some(b)) => {
                if a != b && !a.follows(b) && !b.follows(a) {
                    return None;
                }
                if (a.issued, a.successes.len()) >= (b.issued, b.successes.len()) {
                    a
                } else {
                    b
                }
            }
            (Some(one), None) | (None, Some(one)) if self.fence.as_ref() == Some(one) => one,
            _ => return None,
        };
        if self
            .fence
            .as_ref()
            .is_some_and(|fence| fence != selected && !fence.follows(selected))
        {
            return None;
        }
        Some(selected.clone())
    }
    fn damage(&self, mask: u8) -> Self {
        Self {
            a: (mask & 1 == 0).then(|| self.a.clone()).flatten(),
            b: (mask & 2 == 0).then(|| self.b.clone()).flatten(),
            fence: (mask & 4 == 0).then(|| self.fence.clone()).flatten(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect {
    FencePartial,
    FenceWrite,
    FenceSync,
    ReplaceA,
    ReplaceB,
    DirectorySync,
}
use Effect::*;
const PUBLICATION: [Effect; 7] = [
    FencePartial,
    FenceWrite,
    FenceSync,
    ReplaceA,
    DirectorySync,
    ReplaceB,
    DirectorySync,
];
const REPAIR: [Effect; 6] = [
    ReplaceA,
    ReplaceB,
    DirectorySync,
    FencePartial,
    FenceWrite,
    FenceSync,
];

#[derive(Clone)]
struct Storage {
    visible: Image,
    durable: Image,
    fence_dirty: bool,
}
impl Storage {
    fn settled(image: Image) -> Self {
        Self {
            visible: image.clone(),
            durable: image,
            fence_dirty: false,
        }
    }
    fn effect(&mut self, effect: Effect, root: &Root) {
        match effect {
            FencePartial => {
                self.visible.fence = None;
                self.fence_dirty = true;
            }
            FenceWrite => {
                self.visible.fence = Some(root.clone());
                self.fence_dirty = true;
            }
            FenceSync => {
                self.durable.fence = self.visible.fence.clone();
                self.fence_dirty = false;
            }
            ReplaceA => self.visible.a = Some(root.clone()),
            ReplaceB => self.visible.b = Some(root.clone()),
            DirectorySync => {
                self.durable.a = self.visible.a.clone();
                self.durable.b = self.visible.b.clone();
            }
        }
    }
    fn trace(&self, root: &Root, effects: &[Effect]) -> Vec<Self> {
        let mut states = vec![self.clone()];
        for effect in effects {
            let mut next = states.last().unwrap().clone();
            next.effect(*effect, root);
            states.push(next);
        }
        states
    }
    fn crashes(&self) -> BTreeSet<Image> {
        let mut fences = vec![self.visible.fence.clone(), self.durable.fence.clone()];
        if self.fence_dirty {
            fences.push(None);
        }
        let mut images = BTreeSet::new();
        for a in [&self.visible.a, &self.durable.a] {
            for b in [&self.visible.b, &self.durable.b] {
                for fence in &fences {
                    images.insert(Image {
                        a: a.clone(),
                        b: b.clone(),
                        fence: fence.clone(),
                    });
                }
            }
        }
        images
    }
}

fn preserves(known: &Root, selected: &Root) {
    assert!(selected.issued >= known.issued);
    for attempt in &known.successes {
        assert_eq!(selected.resolve(*attempt), known.resolve(*attempt));
    }
}

fn roots() -> Vec<Root> {
    let mut result = Vec::new();
    for issued in 0..=3 {
        for mask in 0..1 << issued {
            result.push(Root {
                issued,
                successes: (1..=issued)
                    .filter(|number| mask & (1 << (number - 1)) != 0)
                    .collect(),
            });
        }
    }
    result
}

fn publication() -> usize {
    let mut observations = 0;
    for old in roots() {
        let mut next = Vec::new();
        if old.issued < 3 {
            next.push(old.issue());
        }
        if old.issued > 0 && !old.successes.contains(&old.issued) {
            next.push(old.commit());
        }
        for new in next {
            let initial = Storage::settled(Image::settled(&old));
            for (phase, state) in initial.trace(&new, &PUBLICATION).iter().enumerate() {
                for crashed in state.crashes() {
                    let selected = crashed.recover().expect("legal crash remains recoverable");
                    assert!(selected == old || selected == new);
                    preserves(&old, &selected);
                    if phase == PUBLICATION.len() {
                        assert_eq!(selected, new);
                    }
                    for damage in 0..8 {
                        let image = crashed.damage(damage);
                        let Some(admitted) = image.recover() else {
                            continue;
                        };
                        preserves(&old, &admitted);
                        if phase == PUBLICATION.len() {
                            assert_eq!(admitted, new);
                        }
                        for repair in Storage::settled(image).trace(&admitted, &REPAIR) {
                            for interrupted in repair.crashes() {
                                assert_eq!(interrupted.recover(), Some(admitted.clone()));
                                observations += 1;
                            }
                        }
                    }
                }
                // Failed I/O leaves the old durable image under newer visible
                // names. Repair must synchronize that difference before returning.
                let selected = state.visible.recover().unwrap();
                let repairs = state.trace(&selected, &REPAIR);
                for repair in &repairs {
                    for interrupted in repair.crashes() {
                        let observed = interrupted.recover().unwrap();
                        assert!(observed == old || observed == new);
                        preserves(&old, &observed);
                    }
                }
                for crashed in repairs.last().unwrap().crashes() {
                    assert_eq!(crashed.recover(), Some(selected.clone()));
                }
            }
        }
    }
    observations
}

fn identity() {
    // The reference explicitly writes one receipt per attempt, instead of inferring
    // aborts from absence in the compact success list. Enumerate 6^5 event histories.
    for mut history in 0..6usize.pow(5) {
        let mut root = Root::empty();
        let mut receipts = [Receipt::Missing; 5];
        let mut next = 1usize;
        let mut generation = 0;
        let mut exposed = BTreeSet::new();
        for _ in 0..5 {
            let event = history % 6;
            history /= 6;
            // Events: abort, commit, lost issuance, retained issuance,
            // uncertain abort, uncertain commit. Capacity is three attempts.
            let before = root.clone();
            let mut token = None;
            if root.issued < 3 && event != 2 {
                root = root.issue();
                if matches!(event, 1 | 5) {
                    root = root.commit();
                }
                if event != 3 {
                    token = Some(root.issued);
                }
            }
            if next <= 3 && event != 2 {
                assert_eq!(token, (event != 3).then_some(next as u64));
                if let Some(token) = token {
                    assert!(exposed.insert(token));
                }
                receipts[next] = if matches!(event, 1 | 5) {
                    generation += 1;
                    Receipt::Durable(generation)
                } else {
                    Receipt::Aborted
                };
                next += 1;
            } else {
                assert_eq!(token, None);
                assert_eq!(root, before);
            }
            for (attempt, expected) in receipts.iter().enumerate() {
                assert_eq!(root.resolve(attempt as u64), *expected);
            }
            assert_eq!(root.successes.len(), generation);
        }
    }
    let before = Root::empty().issue();
    let after = before.commit();
    let mut failures = Vec::new();
    for first in 0..4 {
        for second in first + 1..4 {
            let mut root = before.clone();
            let mut active = Some(1);
            let mut writes = 0;
            let mut captured = None;
            let mut answer = Receipt::Missing;
            let mut schedule = String::new();
            for step in 0..4 {
                if step == first || step == second {
                    schedule.push('W');
                    if writes == 0 {
                        root = after.clone();
                    } else {
                        active = None;
                    }
                    writes += 1;
                } else {
                    schedule.push('R');
                    if let Some(captured) = &captured {
                        answer = Root::live(captured, active, 1);
                    } else {
                        captured = Some(root.clone());
                    }
                }
                assert!(matches!(
                    root.live(active, 1),
                    Receipt::Pending | Receipt::Durable(1)
                ));
            }
            if answer == Receipt::Aborted {
                failures.push(schedule);
            }
        }
    }
    assert_eq!(failures, ["RWWR"]);
}

fn controls() {
    let empty = Root::empty();
    let issued = empty.issue();
    let committed = issued.commit();
    let clean = Storage::settled(Image::settled(&empty));
    let states = clean.trace(&issued, &PUBLICATION);
    let before_barrier = &states[4];
    assert_eq!(before_barrier.visible.recover(), Some(issued.clone()));
    assert!(
        before_barrier
            .crashes()
            .iter()
            .any(|image| image.recover() == Some(empty.clone()))
    );
    let broken: Vec<_> = REPAIR
        .into_iter()
        .filter(|effect| *effect != DirectorySync)
        .collect();
    let repairs = before_barrier.trace(&issued, &broken);
    assert!(
        repairs
            .last()
            .unwrap()
            .crashes()
            .iter()
            .any(|image| image.recover() != Some(issued.clone()))
    );
    let unfenced: Vec<_> = PUBLICATION
        .into_iter()
        .filter(|effect| *effect != FenceSync)
        .collect();
    assert!(clean.trace(&issued, &unfenced).iter().any(|state| {
        state
            .crashes()
            .iter()
            .any(|image| image.recover().is_none())
    }));
    let lone = Image {
        a: Some(issued.clone()),
        b: None,
        fence: Some(issued.clone()),
    };
    assert_eq!(lone.recover(), Some(issued.clone()));
    assert!(
        Image {
            fence: None,
            ..lone
        }
        .recover()
        .is_none()
    );
    // A later pending fence is not the authority for already committed data.
    assert_eq!(
        Image {
            fence: Some(committed.issue()),
            ..Image::settled(&committed)
        }
        .recover(),
        Some(committed)
    );
    let history = Root {
        issued: 3,
        successes: vec![1, 3],
    };
    assert_ne!(
        history.resolve(1),
        Root {
            issued: 3,
            successes: vec![3]
        }
        .resolve(1)
    );
    // Issuance and generation together order the final u64 attempt without
    // storing their sum or requiring a spare identity for its eventual commit.
    let final_attempt = Root {
        issued: u64::MAX,
        successes: vec![],
    };
    let final_commit = final_attempt.commit();
    assert!(final_commit.follows(&final_attempt));
    assert_eq!(final_commit.resolve(u64::MAX), Receipt::Durable(1));
    assert_eq!(final_commit.resolve(u64::MAX - 1), Receipt::Aborted);
    assert!(final_attempt.issued.checked_add(1).is_none());
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Object {
    Data(usize),
    History(Vec<u64>),
}

fn objects(root: &Root) -> BTreeSet<Object> {
    if root.successes.is_empty() {
        return BTreeSet::new();
    }
    BTreeSet::from([
        Object::Data(root.successes.len()),
        Object::History(root.successes.clone()),
    ])
}

fn reclamation() {
    // Abstract each complete graph as data plus its full receipt index. Reader
    // pins are a coherent input here; the model does not implement their locks.
    for old in roots() {
        if old.issued == 0 || old.successes.contains(&old.issued) {
            continue;
        }
        let new = old.commit();
        let available: BTreeSet<_> = objects(&old).union(&objects(&new)).cloned().collect();
        let states = Storage::settled(Image::settled(&old)).trace(&new, &PUBLICATION);
        let published = states.last().unwrap();
        for pinned in [false, true] {
            let mut protected = objects(&new);
            if pinned {
                protected.extend(objects(&old));
            }
            let retired: Vec<_> = available.difference(&protected).cloned().collect();
            for mask in 0..1 << retired.len() {
                let removed: BTreeSet<_> = retired
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, object)| object.clone())
                    .collect();
                let remaining: BTreeSet<_> = available.difference(&removed).cloned().collect();
                for image in published.crashes() {
                    let selected = image.recover().unwrap();
                    assert!(objects(&selected).is_subset(&remaining));
                    if pinned {
                        assert!(objects(&old).is_subset(&remaining));
                    }
                    preserves(&old, &selected);
                }
            }
        }
    }
    let old = Root {
        issued: 2,
        successes: vec![1],
    };
    let new = old.commit();
    let states = Storage::settled(Image::settled(&old)).trace(&new, &PUBLICATION);
    let remaining = objects(&new);
    assert!(
        states[4]
            .crashes()
            .iter()
            .any(|image| !objects(&image.recover().unwrap()).is_subset(&remaining)),
        "premature deletion control"
    );
    assert!(
        !objects(&old).is_subset(&remaining),
        "ignored reader pin control"
    );
    assert_eq!(new.resolve(1), Receipt::Durable(1));
}

pub fn run() -> Result<()> {
    identity();
    let observations = publication();
    controls();
    reclamation();
    println!(
        "models: 7776 receipt histories; {observations} interrupted repair observations; deliberate missing-barrier and wrong-history controls rejected"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn histories_publication_and_failure_controls() {
        super::run().unwrap();
    }
}
