//! Bandeau de l'accueil : 7 films tirés au sort chaque jour, sans répéter la veille.

use std::path::Path;

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::jellyfin::HeroItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DailyHero {
    pub day: String,
    pub ids: Vec<String>,
}

/// Date du jour (`AAAA-MM-JJ`) dans le fuseau de `site.json`.
pub fn local_day_key(timezone: &str) -> String {
    let now = jiff::Timestamp::now();
    let zoned = jiff::tz::TimeZone::get(timezone).map(|tz| now.to_zoned(tz)).unwrap_or_else(|_| now.to_zoned(jiff::tz::TimeZone::UTC));
    zoned.date().to_string()
}

/// Reprend la sélection du jour si elle existe, sinon en tire une nouvelle en
/// évitant les films de la sélection précédente quand c'est possible.
pub fn select(items: &[HeroItem], saved: Option<&DailyHero>, day: &str, count: usize, rng: &mut impl rand::Rng) -> (DailyHero, Vec<HeroItem>) {
    let wanted = count.min(items.len());
    if let Some(saved) = saved.filter(|s| s.day == day) {
        let selected: Vec<HeroItem> = saved.ids.iter().filter_map(|id| items.iter().find(|item| &item.id == id).cloned()).collect();
        if selected.len() >= wanted {
            return (saved.clone(), selected.into_iter().take(count).collect());
        }
    }
    let previous: Vec<&String> = saved.map(|s| s.ids.iter().collect()).unwrap_or_default();
    let fresh: Vec<&HeroItem> = items.iter().filter(|item| !previous.contains(&&item.id)).collect();
    let mut pool: Vec<&HeroItem> = if fresh.len() >= wanted { fresh } else { items.iter().collect() };
    pool.shuffle(rng);
    let selected: Vec<HeroItem> = pool.into_iter().take(count).cloned().collect();
    let record = DailyHero { day: day.to_string(), ids: selected.iter().map(|i| i.id.clone()).collect() };
    (record, selected)
}

pub fn load(file: &Path) -> Option<DailyHero> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

pub fn daily_selection(file: &Path, items: &[HeroItem], timezone: &str) -> Vec<HeroItem> {
    let day = local_day_key(timezone);
    let saved = load(file);
    let (record, selected) = select(items, saved.as_ref(), &day, 7, &mut rand::thread_rng());
    if saved.as_ref() != Some(&record) {
        let _ = crate::fsutil::atomic_write(file, serde_json::to_string_pretty(&record).unwrap_or_default().as_bytes());
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn items(n: usize) -> Vec<HeroItem> {
        (0..n).map(|i| HeroItem { id: format!("{i:032x}"), name: format!("Film {i}"), ..HeroItem::default() }).collect()
    }

    #[test]
    fn same_day_keeps_selection() {
        let all = items(20);
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let (record, first) = select(&all, None, "2026-11-13", 7, &mut rng);
        let (_, again) = select(&all, Some(&record), "2026-11-13", 7, &mut rng);
        assert_eq!(first, again);
        assert_eq!(first.len(), 7);
    }

    #[test]
    fn next_day_avoids_yesterday() {
        let all = items(20);
        let mut rng = rand::rngs::StdRng::seed_from_u64(2);
        let (record, first) = select(&all, None, "2026-11-13", 7, &mut rng);
        let (_, next) = select(&all, Some(&record), "2026-11-14", 7, &mut rng);
        assert!(next.iter().all(|item| !first.contains(item)));
    }

    #[test]
    fn small_library_reuses_films() {
        let all = items(5);
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let (record, _) = select(&all, None, "2026-11-13", 7, &mut rng);
        let (_, next) = select(&all, Some(&record), "2026-11-14", 7, &mut rng);
        assert_eq!(next.len(), 5);
    }

    #[test]
    fn day_key_has_iso_format() {
        let key = local_day_key("Europe/Paris");
        assert_eq!(key.len(), 10);
        assert_eq!(local_day_key("Pas/Un_Fuseau").len(), 10);
    }
}
