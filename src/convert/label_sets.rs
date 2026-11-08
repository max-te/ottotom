//! Ordering the label sets of a family and dropping repeated ones.

/// A point of a family, with its label set rendered into `Context::attr_buffer`.
#[derive(Clone, Copy)]
pub(super) struct LabeledPoint {
    /// index of the metric in the family's members
    pub(super) member: usize,
    /// index of the point in its metric's data points
    pub(super) point: usize,
    /// byte range of the label set
    pub(super) start: usize,
    pub(super) end: usize,
    /// scratch for [`sort_by_label_set`], meaningless outside of it
    pub(super) key: u64,
}

/// Sorts `points` by label set and keeps one point per label set, then groups
/// the kept points by member.
///
/// Of the points sharing a label set, the one of the first member is kept.
/// Within one member, the SDK's point order is arbitrary, so the point with the
/// smallest [`hash_attrs`](super::labels::hash_attrs) is kept. `point_hashes` gives those hashes for all
/// points of a member, and is called only for such repeats.
// om[impl metricfamily.unique-labelset]
pub(super) fn drop_repeated_label_sets(
    family: &str,
    text: &str,
    points: &mut Vec<LabeledPoint>,
    members: usize,
    point_hashes: impl Fn(usize) -> Vec<u64>,
) {
    sort_by_label_set(text.as_bytes(), points, 0, 0);
    let label_set = |p: &LabeledPoint| &text[p.start..p.end];

    let mut hashes: Vec<Option<Vec<u64>>> = vec![None; members];
    // Survivors are moved to the front, so the runs are tracked by index.
    let mut kept = 0;
    let mut start = 0;
    while start < points.len() {
        let labels = label_set(&points[start]);
        let len = points[start..]
            .iter()
            .take_while(|p| label_set(p) == labels)
            .count();
        let run = &points[start..start + len];
        start += len;

        let first_member = run[0].member;
        let candidates = &run[..run.partition_point(|p| p.member == first_member)];
        let survivor = if let [only] = candidates {
            *only
        } else {
            let member_hashes =
                hashes[first_member].get_or_insert_with(|| point_hashes(first_member));
            *candidates
                .iter()
                .min_by_key(|p| (member_hashes[p.point], p.point))
                .expect("a run is never empty")
        };
        for _ in 1..len {
            // c[impl metadata.drop-warn]
            warn!(
                points = 1,
                "Dropping a point of {family}: its label set {{{labels}}} is already written"
            );
        }
        points[kept] = survivor;
        kept += 1;
    }
    points.truncate(kept);

    // A stable sort allocates scratch space even for sorted input, which is
    // what a family of one metric always is.
    if !points.is_sorted_by_key(|p| p.member) {
        points.sort_by_key(|p| p.member);
    }
}

/// Sorts `points` by label set, and the points of one label set by member.
/// Points of one member with the same label set stay in any order. All label
/// sets in `points` agree on their first `skip` bytes.
///
/// Points are sorted by 8 bytes of their label set at a time. Before each
/// round, the bytes that all label sets of the group share are skipped, so a
/// long common prefix is read once instead of in every comparison.
pub(super) fn sort_by_label_set(
    text: &[u8],
    points: &mut [LabeledPoint],
    skip: usize,
    depth: usize,
) {
    /// Rounds after which the rest is sorted by comparing whole label sets.
    const MAX_DEPTH: usize = 16;

    let Some((first, others)) = points.split_first() else {
        return;
    };
    let after = |p: &LabeledPoint, skip: usize| &text[p.start + skip..p.end];
    let common = others.iter().fold(after(first, skip).len(), |common, p| {
        common_prefix_len(&after(first, skip)[..common], after(p, skip))
    });
    let skip = skip + common;

    if depth == MAX_DEPTH {
        points.sort_unstable_by(|a, b| {
            after(a, skip)
                .cmp(after(b, skip))
                .then(a.member.cmp(&b.member))
        });
        return;
    }

    for point in points.iter_mut() {
        point.key = sort_key(after(point, skip));
    }
    // The key pads short label sets with zeros, and label values may hold NUL
    // bytes. Of two label sets with the same key, the one with fewer bytes
    // left is the smaller, unless both go on past the key.
    let group = |p: &LabeledPoint| (p.key, (p.end - p.start - skip).min(8));
    points.sort_unstable_by_key(|p| (group(p), p.member));

    for run in points.chunk_by_mut(|a, b| group(a) == group(b)) {
        // Label sets that go on past an equal key are ordered by what follows.
        let goes_on = group(&run[0]).1 == 8;
        if run.len() > 1 && goes_on {
            sort_by_label_set(text, run, skip + 8, depth + 1);
        }
    }
}

/// The number of leading bytes `a` and `b` share, compared 8 bytes at a time.
fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    let len = a.len().min(b.len());
    let mut i = 0;
    while i + 8 <= len {
        let word = |bytes: &[u8]| u64::from_le_bytes(bytes[i..i + 8].try_into().expect("8 bytes"));
        let difference = word(a) ^ word(b);
        if difference != 0 {
            return i + difference.trailing_zeros() as usize / 8;
        }
        i += 8;
    }
    i + std::iter::zip(&a[i..len], &b[i..len])
        .take_while(|(x, y)| x == y)
        .count()
}

/// The first 8 bytes of `bytes` as a number, padded with zeros. When the keys
/// of two byte strings differ, they order the strings as their bytes do.
fn sort_key(bytes: &[u8]) -> u64 {
    let mut key = [0; 8];
    let len = bytes.len().min(8);
    key[..len].copy_from_slice(&bytes[..len]);
    u64::from_be_bytes(key)
}
