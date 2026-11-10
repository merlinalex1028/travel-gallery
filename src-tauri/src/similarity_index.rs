//! Exact candidate retrieval for 64-bit perceptual hashes.
use std::collections::HashMap;

pub(crate) struct CandidateIndex {
    radius: u32,
    inner: Index,
}
enum Index {
    Blocks(HammingBlocks),
    Tree(BkTree),
}
impl CandidateIndex {
    pub(crate) fn new(radius: u32) -> Self {
        Self {
            radius,
            inner: if radius <= 8 {
                Index::Blocks(HammingBlocks::default())
            } else {
                Index::Tree(BkTree::default())
            },
        }
    }
    pub(crate) fn add(&mut self, hash: u64, index: usize) {
        match &mut self.inner {
            Index::Blocks(blocks) => blocks.add(hash, index),
            Index::Tree(tree) => tree.add(hash, index),
        }
    }
    pub(crate) fn search(&mut self, hash: u64) -> Vec<usize> {
        match &mut self.inner {
            Index::Blocks(blocks) => blocks.search(hash, self.radius),
            Index::Tree(tree) => tree.search(hash, self.radius),
        }
    }
    #[cfg(test)]
    fn comparisons(&self) -> u64 {
        match &self.inner {
            Index::Blocks(blocks) => blocks.search_comparisons,
            Index::Tree(tree) => tree.search_visits.get(),
        }
    }
    #[cfg(test)]
    fn centers(&self) -> usize {
        match &self.inner {
            Index::Blocks(blocks) => blocks.records.len(),
            Index::Tree(tree) => tree.nodes.iter().map(|(_, indices, _)| indices.len()).sum(),
        }
    }
}

// Nine disjoint blocks cover all64bits (8 + 8*7). At most8changed
// bits leave at least one block identical, so the union loses no match.
// Full Hamming distance still decides inclusion; a matching block is not
// itself a similarity result. Wider radii keep the exact metric tree.
struct HammingBlocks {
    bands: [HashMap<u8, Vec<usize>>; 9],
    records: Vec<(u64, usize)>,
    seen: Vec<u32>,
    generation: u32,
    #[cfg(test)]
    search_comparisons: u64,
}
impl Default for HammingBlocks {
    fn default() -> Self {
        Self {
            bands: std::array::from_fn(|_| HashMap::new()),
            records: vec![],
            seen: vec![],
            generation: 0,
            #[cfg(test)]
            search_comparisons: 0,
        }
    }
}
impl HammingBlocks {
    fn key(hash: u64, band: usize) -> u8 {
        if band == 0 {
            hash as u8
        } else {
            ((hash >> (8 + (band - 1) * 7)) & 0x7f) as u8
        }
    }
    fn add(&mut self, hash: u64, index: usize) {
        let record = self.records.len();
        self.records.push((hash, index));
        self.seen.push(0);
        for (band, buckets) in self.bands.iter_mut().enumerate() {
            buckets
                .entry(Self::key(hash, band))
                .or_default()
                .push(record);
        }
    }
    fn search(&mut self, hash: u64, radius: u32) -> Vec<usize> {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen.fill(0);
            self.generation = 1;
        }
        let mut matches = vec![];
        for (band, buckets) in self.bands.iter().enumerate() {
            if let Some(records) = buckets.get(&Self::key(hash, band)) {
                for record in records {
                    if self.seen[*record] == self.generation {
                        continue;
                    }
                    self.seen[*record] = self.generation;
                    #[cfg(test)]
                    {
                        self.search_comparisons += 1;
                    }
                    let (candidate, index) = self.records[*record];
                    if (candidate ^ hash).count_ones() <= radius {
                        matches.push(index);
                    }
                }
            }
        }
        matches
    }
}

#[derive(Default)]
struct BkTree {
    nodes: Vec<(u64, Vec<usize>, HashMap<u32, usize>)>,
    #[cfg(test)]
    search_visits: std::cell::Cell<u64>,
}
impl BkTree {
    fn add(&mut self, h: u64, index: usize) {
        if self.nodes.is_empty() {
            self.nodes.push((h, vec![index], HashMap::new()));
            return;
        }
        let mut n = 0;
        loop {
            let d = (self.nodes[n].0 ^ h).count_ones();
            if d == 0 {
                self.nodes[n].1.push(index);
                return;
            }
            if let Some(child) = self.nodes[n].2.get(&d) {
                n = *child;
            } else {
                let next = self.nodes.len();
                self.nodes[n].2.insert(d, next);
                self.nodes.push((h, vec![index], HashMap::new()));
                return;
            }
        }
    }
    fn search(&self, h: u64, max: u32) -> Vec<usize> {
        let mut out = vec![];
        if self.nodes.is_empty() {
            return out;
        }
        let mut pending = vec![0];
        while let Some(n) = pending.pop() {
            #[cfg(test)]
            self.search_visits.set(self.search_visits.get() + 1);
            let (hash, ids, children) = &self.nodes[n];
            let d = (*hash ^ h).count_ones();
            if d <= max {
                out.extend(ids.iter().copied());
            }
            for (distance, child) in children {
                if *distance >= d.saturating_sub(max) && *distance <= d + max {
                    pending.push(*child);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod similarity_index_tests {
    use super::*;
    use serde_json::json;
    use std::{io::Write, time::Instant};

    fn hash_at(index: u64) -> u64 {
        let mut value = index.wrapping_add(0x9e3779b97f4a7c15);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    #[test]
    fn indexed_hamming_candidates_match_independent_exhaustive_search() {
        let mut hashes: Vec<_> = (0..1000).map(hash_at).collect();
        hashes.extend([0, u64::MAX, 0, 1, 3, 255, hash_at(42)]);
        let base = hash_at(42);
        for changes in 1..=8 {
            for offset in 0..64 {
                hashes.push(base ^ ((1u64 << changes) - 1).rotate_left(offset));
            }
        }
        for skipped in 0..9 {
            let mut hash = base;
            for band in 0..9 {
                if band != skipped {
                    hash ^= 1 << if band == 0 { 0 } else { 8 + (band - 1) * 7 };
                }
            }
            hashes.push(hash);
        }
        for threshold in (0..=20).chain([64]) {
            let mut index = CandidateIndex::new(threshold);
            for (position, hash) in hashes.iter().enumerate() {
                index.add(*hash, position * 3 + 7);
            }
            for query in hashes.iter().step_by(37).chain([&0, &u64::MAX, &base]) {
                let expected: Vec<_> = hashes
                    .iter()
                    .enumerate()
                    .filter(|(_, hash)| (**hash ^ query).count_ones() <= threshold)
                    .map(|(position, _)| position * 3 + 7)
                    .collect();
                let mut actual = index.search(*query);
                actual.sort_unstable();
                assert_eq!(actual, expected, "query={query:x}, radius={threshold}");
            }
        }
    }

    #[test]
    fn query_generation_wrap_keeps_all_duplicate_centers() {
        let mut blocks = HammingBlocks::default();
        blocks.add(0, 5);
        blocks.add(0, 900);
        blocks.add(1, 7);
        blocks.seen.fill(1);
        blocks.generation = u32::MAX;
        let mut found = blocks.search(0, 1);
        found.sort_unstable();
        assert_eq!(found, [5, 7, 900]);
        assert_eq!(blocks.search(0, 0), [5, 900]);
    }

    #[test]
    #[ignore = "Explicit release-only 100k dense-time-bucket nearest-index measurement"]
    fn dense_hundred_thousand_index_baseline() {
        assert!(
            !cfg!(debug_assertions),
            "Measure an optimized release build"
        );
        let output = std::path::PathBuf::from(
            std::env::var_os("TRAVEL_GALLERY_SIMILAR_INDEX_OUTPUT")
                .expect("provide a new absolute JSON report path"),
        );
        assert!(output.is_absolute() && output.parent().unwrap().is_dir());
        let mut report = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .unwrap();
        let mut rows = vec![];
        for count in [10_000, 50_000, 100_000] {
            let started = Instant::now();
            let mut tree = CandidateIndex::new(8);
            let mut samples = Vec::with_capacity(count);
            let mut candidates = 0;
            for index in 0..count {
                let hash = hash_at(index as u64);
                let queried = Instant::now();
                let found = tree.search(hash);
                samples.push(queried.elapsed().as_secs_f64() * 1000.);
                candidates += found.len();
                // Uniform 64-bit hashes rarely match at radius8, so almost all
                // become centers. This deliberately stresses one dense time bucket.
                if found.is_empty() {
                    tree.add(hash, index);
                }
                if (index + 1) % 10_000 == 0 {
                    println!(
                        "SIMILAR_INDEX_PROGRESS count={count} processed={} elapsedSeconds={:.3}",
                        index + 1,
                        started.elapsed().as_secs_f64()
                    );
                    std::io::stdout().flush().unwrap();
                }
            }
            samples.sort_by(f64::total_cmp);
            rows.push(json!({"entries":count,"centers":tree.centers(),
                "elapsedSeconds":started.elapsed().as_secs_f64(),
                "candidateComparisons":tree.comparisons(),"candidateResults":candidates,
                "queryP50Ms":samples[(count / 2) - 1],
                "queryP95Ms":samples[(count * 95 / 100) - 1],
                "queryMaxMs":samples[count - 1]}));
        }
        let value = json!({"atUtc":chrono::Utc::now().to_rfc3339(),
            "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "algorithm":"production CandidateIndex nine-block exact candidate union; default Hamming radius8",
            "dataset":"deterministic SplitMix64 hashes; all in one time bucket; no decoded media",
            "percentiles":"nearest rank, sorted[ceil(n*q)-1]",
            "runs":rows,"limits":[
                "Index-only measurement excludes decoding, SQLite, job safe points, source checks and group publication.",
                "Test-only distance-comparison counter is enabled; normal application has no counter.",
                "One dense bucket stresses the default index; other radii, distributions and real device quality require separate evidence.",
                "No source photos or normal application catalog are opened or modified."]});
        serde_json::to_writer_pretty(&mut report, &value).unwrap();
        report.write_all(b"\n").unwrap();
        report.sync_all().unwrap();
        println!("SIMILAR_INDEX_RESULT={value}");
    }
}
