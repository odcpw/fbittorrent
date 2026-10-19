//! Session card storage is keyed by the resolved IMDb ID, with bounded aliases.
use super::*;
impl TuiApp {
    pub(super) fn imdb_state(&self, name: &str) -> Option<&ImdbState> {
        self.imdb
            .get(self.imdb_aliases.get(name).map_or(name, String::as_str))
    }
    pub(super) fn cache_imdb(&mut self, key: String, state: ImdbState) {
        if self.imdb.len() >= 256 && !self.imdb.contains_key(&key) {
            if let Some(old) = self
                .imdb
                .iter()
                .find(|(_, state)| !matches!(state, ImdbState::Loading))
                .map(|(key, _)| key.clone())
                .or_else(|| self.imdb.keys().next().cloned())
            {
                self.imdb.remove(&old);
                self.imdb_aliases.retain(|_, v| v != &old);
            }
        }
        self.imdb.insert(key, state);
    }
    pub(super) fn remember_imdb(
        &mut self,
        key: String,
        result: Result<Option<(Title, Option<Poster>)>, String>,
    ) {
        let (canonical, mut state) = match result {
            Ok(Some((title, poster))) => {
                (title.id.clone(), ImdbState::Ready(Box::new(title), poster))
            }
            Ok(None) => (key.clone(), ImdbState::Missing),
            Err(error) => (key.clone(), ImdbState::Failed(error)),
        };
        if canonical != key {
            self.imdb.remove(&key);
            for alias in self.imdb_aliases.values_mut() {
                if alias == &key {
                    alias.clone_from(&canonical);
                }
            }
        }
        if matches!(self.imdb.get(&canonical), Some(ImdbState::Ready(..)))
            && !matches!(state, ImdbState::Ready(..))
        {
            return;
        }
        if let ImdbState::Ready(_, poster) = &mut state {
            if poster.is_none() {
                if let Some(ImdbState::Ready(_, old)) = self.imdb.remove(&canonical) {
                    *poster = old;
                }
            }
        }
        self.cache_imdb(canonical, state);
    }
}
