use sr_geo::pmtiles::{decompress_within, parse_directory};

#[test]
fn uncompressed_and_mislabeled_gzip_payloads_obey_the_requested_bound() {
    assert!(decompress_within(1, vec![0; 10], 9).is_err());
    assert!(decompress_within(2, vec![0; 10], 9).is_err());
    assert_eq!(decompress_within(1, vec![1; 9], 9).unwrap(), vec![1; 9]);
}

#[test]
fn directory_columns_cannot_silently_truncate_to_u32() {
    for bytes in [vec![1, 0, 128, 128, 128, 128, 16, 1, 1], vec![1, 0, 1, 128, 128, 128, 128, 16, 1]] {
        assert!(parse_directory(&bytes).is_err());
    }
}
