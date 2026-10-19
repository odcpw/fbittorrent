//! Acknowledgement bookkeeping is separate from the confirmed torrent mirror.
use super::*;

const MAX_PENDING: usize = 64;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tui) enum Status {
    Waiting,
    Unknown,
}
#[derive(Debug)]
pub(in crate::tui) struct Pending {
    pub op: String,
    pub hashes: Vec<api::Hash>,
    pub status: Status,
}
impl Store {
    pub fn begin_mutation(
        &mut self,
        reference: api::MutationRef,
        op: String,
        hashes: Vec<api::Hash>,
    ) -> Result<()> {
        api::validation::text(&op, 128, "mutation.op")?;
        ensure!(
            self.pending.len() < MAX_PENDING,
            "64 mutations are already awaiting confirmation"
        );
        ensure!(
            hashes.len() <= api::MAX_HASHES,
            "mutation exceeds the hash bound"
        );
        ensure!(
            !self.pending.contains_key(&reference),
            "mutation reference already pending"
        );
        self.pending.insert(
            reference,
            Pending {
                op,
                hashes,
                status: Status::Waiting,
            },
        );
        Ok(())
    }
    pub(in crate::tui) fn mutation_failed(&mut self, reference: &api::MutationRef, unknown: bool) {
        if unknown {
            if let Some(pending) = self.pending.get_mut(reference) {
                pending.status = Status::Unknown;
            }
        } else {
            self.pending.remove(reference);
        }
    }
    pub fn pending_for_key(&self, hash: &str) -> Option<&str> {
        self.pending
            .values()
            .find(|pending| {
                pending.status == Status::Waiting
                    && pending.hashes.iter().any(|key| key.as_str() == hash)
            })
            .map(|pending| pending.op.as_str())
    }
    pub(super) fn fresh_rows(&mut self) {
        self.pending
            .retain(|_, pending| pending.status != Status::Unknown);
    }
    pub(super) fn lose_pending(&mut self) {
        for pending in self.pending.values_mut() {
            pending.status = Status::Unknown;
        }
    }
    pub(super) fn settle(&mut self, event: &api::Event) {
        if let api::Event::MutationResult {
            reference, outcome, ..
        } = event
        {
            match outcome {
                api::MutationOutcome::Timeout => {
                    if let Some(pending) = self.pending.get_mut(reference) {
                        pending.status = Status::Unknown;
                    }
                }
                api::MutationOutcome::Done | api::MutationOutcome::Failed => {
                    self.pending.remove(reference);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_and_confirmed_mutations_do_not_fabricate_torrent_state() {
        let hash = api::Hash::new("a".repeat(40)).unwrap();
        let reference = api::MutationRef::new("tui-stop-1").unwrap();
        let mut store = Store::default();
        store
            .begin_mutation(
                reference.clone(),
                "torrents.stop".into(),
                vec![hash.clone()],
            )
            .unwrap();
        assert_eq!(store.pending_for_key(hash.as_str()), Some("torrents.stop"));
        assert!(
            store.torrents.is_empty(),
            "pending must never invent the expected end state"
        );
        store
            .event(&api::Event::MutationResult {
                seq: 1,
                ts: 1,
                reference,
                op: "torrents.stop".into(),
                outcome: api::MutationOutcome::Done,
                error: None,
                result: None,
            })
            .unwrap();
        assert!(store.pending_for_key(hash.as_str()).is_none());
        assert!(
            store.torrents.is_empty(),
            "even completion events leave rows to authoritative data"
        );
    }
    #[test]
    fn connection_loss_marks_result_unknown_without_retrying_or_holding_the_row_busy() {
        let hash = api::Hash::new("a".repeat(40)).unwrap();
        let reference = api::MutationRef::new("tui-remove-1").unwrap();
        let mut store = Store::default();
        store
            .begin_mutation(
                reference.clone(),
                "torrents.remove".into(),
                vec![hash.clone()],
            )
            .unwrap();
        store.disconnected();
        assert_eq!(store.pending[&reference].status, Status::Unknown);
        assert!(store.pending_for_key(hash.as_str()).is_none());
        assert!(store.torrents.is_empty());
    }
}
