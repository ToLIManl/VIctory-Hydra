//! Compares that read a declared range whole only once in a while
//! (`VICTORY_CARD_COMPARE_EVERY=<D>`; off by default).
//!
//! After each submission the card compares every 64 KiB piece of every range
//! its shaders were declared to write (`CARD_COMPARE`). Of a copy the title
//! declares whole again and again (its constants buffer: 182 MB, some seven
//! times a frame) that is gigabytes read a frame, of which a few dozen pieces
//! are found changed, and the same few frame after frame. With the switch a
//! declared range is read whole by a compare at most once in `D` submissions;
//! the compares between read only its pieces found changed lately
//! ([`hot_span`] submissions).
//!
//! Nothing is owed and no flight waits: a shader's write to a piece nobody
//! found changed lately reaches title memory with the range's next whole
//! compare, at most `D` submissions after its own flight landed. What may not
//! happen is a piece's stored hash being replaced before a compare looked at
//! it: the hash passes after title bytes are sent up (mode 0) store the hash
//! of what the piece holds then, the shaders' bytes with it, and no compare
//! would flag them again. [`Lean::ahead`] says which pieces a send is about to
//! hash again that a compare left; the host compares those first.
//!
//! The book is pure: the host keeps it under a lock ([`super::LEAN`]), records
//! the compares it asks for and tells it what compares found.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// `VICTORY_CARD_COMPARE_HOT_SPAN` (90, about three frames): for how many
/// submissions after a compare found a piece changed the piece is compared
/// after every submission that declares it.
pub(super) fn hot_span() -> u64 {
    static SPAN: OnceLock<u64> = OnceLock::new();
    *SPAN.get_or_init(|| {
        std::env::var("VICTORY_CARD_COMPARE_HOT_SPAN").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(90)
    })
}

/// `VICTORY_CARD_COMPARE_EVERY=<D>` (off by default; needs the relay): a
/// declared range of a copy is read whole by a compare at most once in `D`
/// submissions; the compares between read only its pieces found changed in
/// the last [`hot_span`] submissions ([`Lean`]).
pub(super) fn every() -> Option<u64> {
    static EVERY: OnceLock<Option<u64>> = OnceLock::new();
    *EVERY.get_or_init(|| {
        std::env::var("VICTORY_CARD_COMPARE_EVERY").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&d| d > 0)
    })
}

/// With [`every`]: whether the pieces a compare left that a send is about to
/// hash again are compared first ([`Lean::ahead`]; on by default,
/// `VICTORY_CARD_COMPARE_EVERY_AHEAD=0` leaves them out, to measure what they
/// cost: shaders' writes to such pieces may then never reach title memory).
pub(super) fn ahead_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_COMPARE_EVERY_AHEAD").is_none_or(|v| v != "0"))
}

/// What [`every`] goes by: for each declared range the submission whose
/// compare last read all of it, and for each copy the pieces found changed
/// and when. A range is a copy, its first piece and its pieces: the title
/// declares the same ranges frame after frame.
///
/// And the ranges a compare left pieces of since their last whole read
/// (`cold`). A shader may have written such a piece, and its stored hash is
/// still the one from before: the range's next whole compare finds the
/// write. But the hash passes after title bytes are sent up (mode 0) store
/// the hash of what the piece holds then, the shader's bytes with it, and no
/// compare would flag them again: they would never reach title memory. So
/// before a submission's sends the pieces they hash again that lie in such a
/// range are compared first ([`Lean::ahead`]), each once until the range is
/// declared again: a few hundred pieces a frame, and nothing kept per piece
/// of a whole range.
#[derive(Default)]
pub(super) struct Lean {
    whole: HashMap<(u64, u32, u32), u64>,
    changed: HashMap<u64, HashMap<u32, u64>>,
    /// Per copy, per range left in part (first piece, pieces): its offsets
    /// in the copy and the pieces compared ahead of sends since it was last
    /// declared.
    cold: HashMap<u64, HashMap<(u32, u32), Cold>>,
    plans: u64,
}

/// A range a compare left pieces of ([`Lean`]).
struct Cold {
    from: u32,
    to: u32,
    read: HashSet<u32>,
}

/// Since the last frame-rate line ([`lean_line`]): declared ranges read
/// whole, ranges read in their hot pieces only, the pieces read of those,
/// the pieces left; pieces compared ahead of sends that hash them again
/// ([`Lean::ahead`]), of those found changed, and fetched whole for want of
/// room in the compare's results.
pub(crate) static LEAN_STATS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

pub(crate) mod lean_stat {
    pub(crate) const AHEAD: usize = 4;
    pub(crate) const AHEAD_FOUND: usize = 5;
    pub(crate) const AHEAD_UNCOMPARED: usize = 6;
}

impl Lean {
    /// What submission `number`'s compare reads of the declared range
    /// (`copy`, from piece `first`, `pieces` of them, offsets `bytes` of the
    /// copy): `None` for all of it, else each piece's yes or no. Called
    /// after [`Lean::ahead`] for the same submission's sends.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan(
        &mut self,
        number: u64,
        every: u64,
        span: u64,
        copy: u64,
        first: u32,
        pieces: u32,
        bytes: (u32, u32),
    ) -> Option<Vec<bool>> {
        self.plans += 1;
        if self.plans.is_multiple_of(1024) {
            self.prune(number, every);
        }
        let whole = self.whole.entry((copy, first, pieces)).or_insert(0);
        if *whole == 0 || *whole + every <= number {
            *whole = number;
            // (What changed long ago is forgotten at these.)
            if let Some(changed) = self.changed.get_mut(&copy) {
                changed.retain(|_, n| *n + 4 * span >= number);
            }
            self.settled(copy, first, pieces);
            LEAN_STATS[0].fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let mut chosen = vec![false; pieces as usize];
        let mut hot = 0u64;
        if let Some(changed) = self.changed.get(&copy) {
            for (&p, &n) in changed {
                if n + span >= number && p >= first && p - first < pieces {
                    chosen[(p - first) as usize] = true;
                    hot += 1;
                }
            }
        }
        if hot < u64::from(pieces) {
            // Left in part: what this submission's shaders write there is
            // compared ahead of any send that hashes it again before the
            // range is next read whole.
            let cold = self.cold.entry(copy).or_default();
            let c = cold.entry((first, pieces)).or_insert_with(|| Cold { from: bytes.0, to: bytes.1, read: HashSet::new() });
            (c.from, c.to) = bytes;
            c.read.clear();
        } else {
            // Read all of it after this submission's lists, as a whole read.
            self.settled(copy, first, pieces);
        }
        LEAN_STATS[1].fetch_add(1, Ordering::Relaxed);
        LEAN_STATS[2].fetch_add(hot, Ordering::Relaxed);
        LEAN_STATS[3].fetch_add(u64::from(pieces) - hot, Ordering::Relaxed);
        Some(chosen)
    }

    /// A compare reads all of the range after this submission's lists: what
    /// earlier ones left of it is found there.
    fn settled(&mut self, copy: u64, first: u32, pieces: u32) {
        if let Some(cold) = self.cold.get_mut(&copy) {
            cold.remove(&(first, pieces));
            if cold.is_empty() {
                self.cold.remove(&copy);
            }
        }
    }

    /// Ahead of a submission's sends to `copy`, which hash its pieces in
    /// `runs` again (`[a, b)` of pieces; `None`: all of them): those of them
    /// that lie in a range a compare left in part since its last whole read,
    /// and that no compare ahead of a send has read since the range was last
    /// declared, each with the offsets of the copy those ranges take (from,
    /// to: the piece's own are the caller's to cut). They are to be compared,
    /// storing their hashes, before the sends; taken as read here.
    pub(super) fn ahead(&mut self, copy: u64, runs: Option<&[(u32, u32)]>) -> Vec<(u32, u32, u32)> {
        let Some(cold) = self.cold.get_mut(&copy) else {
            return Vec::new();
        };
        let mut out: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
        for (&(first, pieces), c) in cold.iter_mut() {
            let (lo, hi) = (first, first + pieces);
            let mut take = |p: u32| {
                if c.read.insert(p) {
                    let at = out.entry(p).or_insert((c.from, c.to));
                    *at = (at.0.min(c.from), at.1.max(c.to));
                }
            };
            match runs {
                None => (lo..hi).for_each(&mut take),
                Some(runs) => {
                    for &(a, b) in runs {
                        (a.max(lo)..b.min(hi)).for_each(&mut take);
                    }
                }
            }
        }
        let out: Vec<(u32, u32, u32)> = out.into_iter().map(|(p, (from, to))| (p, from, to)).collect();
        LEAN_STATS[lean_stat::AHEAD].fetch_add(out.len() as u64, Ordering::Relaxed);
        out
    }

    /// Ranges not declared for long go, and copies with nothing found lately
    /// (their numbers are never given again; the book would only grow). A
    /// range left in part stays until it is read whole: a write there may
    /// still be on the card only.
    fn prune(&mut self, number: u64, every: u64) {
        let cold = &self.cold;
        self.whole.retain(|&(copy, first, pieces), n| {
            *n + 64 * every >= number || cold.get(&copy).is_some_and(|c| c.contains_key(&(first, pieces)))
        });
        self.changed.retain(|_, c| !c.is_empty());
    }

    /// Submission `number`'s compare found `piece` of `copy` changed.
    pub(super) fn found(&mut self, copy: u64, piece: u32, number: u64) {
        let at = self.changed.entry(copy).or_default().entry(piece).or_insert(number);
        *at = (*at).max(number);
    }
}

/// [`LEAN_STATS`], as they stand (nothing is reset): for telemetry that must
/// not take a lock.
pub(crate) fn snapshot() -> [u64; 7] {
    std::array::from_fn(|i| LEAN_STATS[i].load(Ordering::Relaxed))
}

/// The frame-rate line's part ([`LEAN_STATS`]), if the switch is on.
pub(super) fn lean_line(frames: f64) {
    let Some(every) = every() else {
        return;
    };
    let s: Vec<f64> = LEAN_STATS.iter().map(|n| n.swap(0, Ordering::Relaxed) as f64 / frames).collect();
    tracing::info!(
        "declared ranges read whole once in {every} submissions (VICTORY_CARD_COMPARE_EVERY, hot for {} submissions) per frame: {:.1} read whole, \
         {:.1} read in their hot pieces only, {:.0} pieces read of those, {:.0} left; {:.1} pieces left so compared ahead of \
         sends that hash them again, {:.1} of them found changed, {:.1} fetched whole for want of room",
        hot_span(),
        s[0],
        s[1],
        s[2],
        s[3],
        s[lean_stat::AHEAD],
        s[lean_stat::AHEAD_FOUND],
        s[lean_stat::AHEAD_UNCOMPARED],
    );
}

#[cfg(test)]
mod tests {
    use super::Lean;

    #[test]
    fn a_range_is_read_whole_once_in_so_many_submissions_and_in_its_hot_pieces_between() {
        let mut lean = Lean::default();
        // (The offsets of the range: only `Lean::ahead` gives them back.)
        const B: (u32, u32) = (0, 0);
        // The first time, and again when the count has passed: whole.
        assert_eq!(lean.plan(10, 30, 90, 7, 100, 8, B), None);
        assert_eq!(lean.plan(11, 30, 90, 7, 100, 8, B), Some(vec![false; 8]), "nothing found changed yet");
        // Another range of the same copy has its own count.
        assert_eq!(lean.plan(11, 30, 90, 7, 0, 4, B), None);
        // A piece found changed is read by every compare for the span, in the ranges that hold it.
        lean.found(7, 103, 11);
        lean.found(7, 2, 11);
        let mut hot = vec![false; 8];
        hot[3] = true;
        assert_eq!(lean.plan(12, 30, 90, 7, 100, 8, B), Some(hot.clone()));
        assert_eq!(lean.plan(39, 30, 90, 7, 100, 8, B), Some(hot.clone()));
        assert_eq!(lean.plan(40, 30, 90, 7, 100, 8, B), None, "thirty submissions after the last whole one");
        assert_eq!(lean.plan(41, 30, 90, 7, 100, 8, B), Some(hot));
        // Past the span it is cold again.
        assert_eq!(lean.plan(41 + 61, 30, 90, 7, 100, 8, B), None);
        assert_eq!(lean.plan(41 + 62, 30, 90, 7, 100, 8, B), Some(vec![false; 8]));
        // Another copy knows nothing of it.
        assert_eq!(lean.plan(12, 30, 90, 8, 100, 8, B), None);
    }

    /// One piece in the model: the shader's bytes and the title's bytes in
    /// it, each a version.
    type Cells = (u64, u64);

    /// The lean compare ([`Lean`], `VICTORY_CARD_COMPARE_EVERY`) with the
    /// relay, as `Host::card_before` and the landing thread go with it:
    /// fixed declared ranges of one copy, one or two a submission; the title
    /// writes its own bytes of a piece now and then, and the next sends take
    /// them up and hash the piece again; the lists write declared pieces; the
    /// submission's compare reads what `Lean::plan` chose, storing hashes;
    /// flights land some submissions later and fetch what their compares
    /// flagged, the shaders' bytes going into title memory. The card runs
    /// everything at once in queue order: what matters here is the stored
    /// hashes, not the timing. `fix`: the pieces the sends hash again are
    /// compared first as `Lean::ahead` says, their flags landing with the
    /// submission.
    ///
    /// Checked at every landing of a flight whose compare read a range
    /// whole: title memory holds every write shaders made to that range's
    /// pieces in that flight and before (a write lands at most with the
    /// range's next whole read). At the end, with everything read whole once
    /// more and nothing sent, title memory holds every write. Returns the
    /// landings, or what went wrong.
    fn lean_model(seed: u64, every: u64, fix: bool) -> Result<u32, String> {
        const PIECES: usize = 10;
        const RANGES: [(u32, u32); 4] = [(0, 10), (2, 3), (5, 4), (8, 2)];
        type Line = std::collections::VecDeque<(u64, Vec<u32>, Vec<(u32, u32)>)>;
        let mut seed = seed | 1;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut lean = Lean::default();
        let (mut card, mut stored, mut held, mut title) =
            ([(0u64, 0u64); PIECES], [(0, 0); PIECES], [(0, 0); PIECES], [(0, 0); PIECES]);
        let compare = |pieces: &[u32], card: &[Cells; PIECES], stored: &mut [Cells; PIECES]| -> Vec<u32> {
            let flagged = pieces.iter().copied().filter(|&p| card[p as usize] != stored[p as usize]).collect();
            for &p in pieces {
                stored[p as usize] = card[p as usize];
            }
            flagged
        };
        let mut version = 0u64;
        let mut dirty: Vec<u32> = Vec::new();
        // Every shader write: piece, version, flight.
        let mut writes: Vec<(u32, u64, u64)> = Vec::new();
        // Flights in the line: number, flagged pieces, ranges read whole.
        let mut line: Line = Default::default();
        let mut landings = 0;
        let mut land = |line: &mut Line,
                        card: &[Cells; PIECES],
                        held: &mut [Cells; PIECES],
                        title: &mut [Cells; PIECES],
                        lean: &mut Lean,
                        writes: &[(u32, u64, u64)]|
         -> Result<(), String> {
            while let Some((flight, flagged, wholes)) = line.pop_front() {
                for &p in &flagged {
                    let p = p as usize;
                    if card[p].0 != held[p].0 {
                        (title[p].0, held[p].0) = (card[p].0, card[p].0);
                    }
                    lean.found(1, p as u32, flight);
                }
                for &(first, pieces) in &wholes {
                    let late = writes
                        .iter()
                        .find(|&&(p, v, by)| by <= flight && first <= p && p < first + pieces && title[p as usize].0 < v);
                    if let Some((p, v, by)) = late {
                        return Err(format!(
                            "flight {flight} read piece {first}+{pieces} whole and landed, flight {by}'s write {v} of piece {p} is not in title memory"
                        ));
                    }
                }
                landings += 1;
            }
            Ok(())
        };
        let mut number = 0u64;
        for _ in 0..600 {
            number += 1;
            // The title writes its bytes of a piece: into title memory and
            // the CPU's side, to go up with the next send.
            if rand(3) != 0 {
                let p = rand(PIECES as u64) as u32;
                version += 1;
                (title[p as usize].1, held[p as usize].1) = (version, version);
                if !dirty.contains(&p) {
                    dirty.push(p);
                }
            }
            let mut flagged = Vec::new();
            // Ahead of the sends, the pieces they hash again that a compare left.
            if fix && !dirty.is_empty() {
                let runs: Vec<(u32, u32)> = dirty.iter().map(|&p| (p, p + 1)).collect();
                let ahead: Vec<u32> = lean.ahead(1, Some(&runs)).into_iter().map(|a| a.0).collect();
                flagged.extend(compare(&ahead, &card, &mut stored));
            }
            // The sends hash the pieces again.
            for p in dirty.drain(..).map(|p| p as usize) {
                card[p].1 = held[p].1;
                stored[p] = card[p];
            }
            let mut ranges: Vec<(u32, u32)> = (0..1 + rand(2)).map(|_| RANGES[rand(RANGES.len() as u64) as usize]).collect();
            ranges.dedup();
            let mut read: Vec<u32> = Vec::new();
            let mut wholes = Vec::new();
            for &(first, pieces) in &ranges {
                let bytes = (first * 65536, (first + pieces) * 65536);
                match lean.plan(number, every, 6, 1, first, pieces, bytes) {
                    None => {
                        read.extend(first..first + pieces);
                        wholes.push((first, pieces));
                    }
                    Some(chosen) => read.extend((first..first + pieces).zip(chosen).filter(|(_, c)| *c).map(|(p, _)| p)),
                }
            }
            read.sort_unstable();
            read.dedup();
            // The lists write some declared pieces.
            for &(first, pieces) in &ranges {
                for p in first..first + pieces {
                    if rand(4) == 0 {
                        version += 1;
                        card[p as usize].0 = version;
                        writes.push((p, version, number));
                    }
                }
            }
            flagged.extend(compare(&read, &card, &mut stored));
            line.push_back((number, flagged, wholes));
            if rand(3) == 0 {
                land(&mut line, &card, &mut held, &mut title, &mut lean, &writes)?;
            }
        }
        // Every range read whole once more, nothing sent.
        for _ in 0..2 {
            number += every;
            let mut read = Vec::new();
            let mut wholes = Vec::new();
            for &(first, pieces) in &RANGES {
                if lean.plan(number, every, 6, 1, first, pieces, (first * 65536, (first + pieces) * 65536)).is_none() {
                    read.extend(first..first + pieces);
                    wholes.push((first, pieces));
                }
            }
            read.sort_unstable();
            read.dedup();
            let flagged = compare(&read, &card, &mut stored);
            line.push_back((number, flagged, wholes));
            land(&mut line, &card, &mut held, &mut title, &mut lean, &writes)?;
        }
        if let Some(p) = (0..PIECES).find(|&p| title[p].0 != card[p].0) {
            return Err(format!("at the end piece {p} holds the shaders' {} on the card and {} in title memory", card[p].0, title[p].0));
        }
        Ok(landings)
    }

    /// With the compares ahead of the sends no write is lost and none lands
    /// later than its range's next whole read; without them the model sees
    /// writes the sends' hashes took in, never to land.
    #[test]
    fn no_write_is_lost_to_a_send_with_ranges_read_whole_once_in_so_many() {
        let seeds = [0x9e37_79b9_7f4a_7c15u64, 0x2545_f491_4f6c_dd1d, 0xd1b5_4a32_d192_ed03, 7, 12345, 99, 31337];
        for every in [1u64, 2, 3, 5, 8, 30] {
            for &seed in &seeds {
                let landed = lean_model(seed, every, true);
                assert!(landed.as_ref().is_ok_and(|&n| n > 100), "every {every}, seed {seed}: {landed:?}");
            }
        }
        let caught = seeds.iter().any(|&seed| [2u64, 3, 5, 8, 30].iter().any(|&every| lean_model(seed, every, false).is_err()));
        assert!(caught, "the model sees a write lost to a send without the compares ahead of it");
    }

    /// `Lean::ahead`: of a range left in part, the pieces the sends hash
    /// again, each once until the range is declared again; none once the
    /// range was read whole, or all of it was hot.
    #[test]
    fn pieces_left_by_a_compare_are_compared_ahead_of_a_send_once() {
        let mut lean = Lean::default();
        assert_eq!(lean.plan(1, 30, 90, 7, 10, 8, (100, 900)), None);
        assert!(lean.ahead(7, Some(&[(0, 100)])).is_empty(), "read whole after its lists");
        assert_eq!(lean.plan(2, 30, 90, 7, 10, 8, (100, 900)), Some(vec![false; 8]));
        assert!(lean.ahead(8, None).is_empty(), "another copy");
        assert_eq!(
            lean.ahead(7, Some(&[(0, 11), (12, 14), (17, 40)])),
            vec![(10, 100, 900), (12, 100, 900), (13, 100, 900), (17, 100, 900)]
        );
        assert!(lean.ahead(7, Some(&[(10, 11)])).is_empty(), "compared already");
        // Declared again: its pieces may be written again.
        assert_eq!(lean.plan(3, 30, 90, 7, 10, 8, (100, 900)), Some(vec![false; 8]));
        assert_eq!(lean.ahead(7, Some(&[(10, 11)])), vec![(10, 100, 900)]);
        // Two ranges hold a piece: the offsets of both.
        assert_eq!(lean.plan(3, 30, 90, 7, 12, 2, (2000, 3000)), None);
        assert_eq!(lean.plan(4, 30, 90, 7, 12, 2, (2000, 3000)), Some(vec![false; 2]));
        assert_eq!(lean.ahead(7, Some(&[(12, 13)])), vec![(12, 100, 3000)]);
        // All of it hot: read after its lists, nothing left.
        lean.found(7, 12, 4);
        lean.found(7, 13, 4);
        assert_eq!(lean.plan(5, 30, 90, 7, 12, 2, (2000, 3000)), Some(vec![true; 2]));
        assert_eq!(lean.ahead(7, Some(&[(13, 14)])), vec![(13, 100, 900)]);
        // Read whole: nothing left of it.
        assert_eq!(lean.plan(32, 30, 90, 7, 10, 8, (100, 900)), None);
        assert!(lean.ahead(7, None).is_empty());
    }
}
