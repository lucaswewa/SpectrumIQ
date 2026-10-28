use serde_json::json;

use super::client::parts;
use super::{Histogram, percentile};

#[test]
fn every_part_is_counted_once_however_the_stream_is_split() {
    let stream =
        b"HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\n\r\n\
        --frame\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8one\xff\xd9\r\n\
        --frame\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8two\xff\xd9\r\n\
        --frame\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8three";
    for split in 0..stream.len() {
        let mut pending = Vec::new();
        let (first, second) = stream.split_at(split);
        let counted = parts(&mut pending, first) + parts(&mut pending, second);
        assert_eq!(counted, 3, "split at {split}");
    }
    // A byte at a time.
    let mut pending = Vec::new();
    let counted: usize = stream.chunks(1).map(|byte| parts(&mut pending, byte)).sum();
    assert_eq!(counted, 3);
}

#[test]
fn a_quantile_is_the_bound_of_the_bucket_it_falls_in_since_the_first_sample() {
    let histogram = |counts: [u64; 4]| {
        let bounds = [Some(10.0), Some(35.0), Some(100.0), None];
        let buckets: Vec<_> = (bounds.iter().zip(counts))
            .map(|(le, count)| json!({ "le": le, "count": count }))
            .collect();
        let total: u64 = counts.iter().sum();
        Histogram::of(&json!({ "count": total, "sum": 0.0, "buckets": buckets }))
    };
    let before = histogram([50, 0, 0, 0]);
    // Since `before`: 90 values up to 10 ms, 8 to 35 and 2 to 100.
    let after = histogram([140, 8, 2, 0]);
    assert_eq!(after.quantile_since(&before, 0.5), Some(10.0));
    assert_eq!(after.quantile_since(&before, 0.95), Some(35.0));
    assert_eq!(after.quantile_since(&before, 1.0), Some(100.0));
    // Past the last bound, or with nothing new, there is no bound.
    assert_eq!(histogram([50, 0, 0, 5]).quantile_since(&before, 0.95), None);
    assert_eq!(before.quantile_since(&before, 0.95), None);
}

#[test]
fn a_percentile_is_a_value_at_or_above_its_share() {
    let values: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&values, 0.95), 95.0);
    assert_eq!(percentile(&values, 0.5), 50.0);
    assert_eq!(percentile(&[3.0], 0.95), 3.0);
    assert_eq!(percentile(&[], 0.95), 0.0);
}
