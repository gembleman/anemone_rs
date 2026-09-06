use super::*;

#[test]
fn bitmap_bounds_include_layout_overhang_outline_and_directional_shadow() {
    let bounds = compute_outline_bitmap_bounds(
        100.0,
        40.0,
        DWRITE_OVERHANG_METRICS {
            left: 3.25,
            top: 1.5,
            right: 4.75,
            bottom: 2.25,
        },
        6.0,
        -2.0,
        3.0,
    );

    assert_eq!(bounds.layout_origin_x, 11.25);
    assert_eq!(bounds.layout_origin_y, 7.5);
    assert_eq!(bounds.width, 122.0);
    assert_eq!(bounds.height, 59.0);
}

#[test]
fn bitmap_bounds_keep_at_least_one_pixel_for_empty_layout() {
    let bounds =
        compute_outline_bitmap_bounds(0.0, 0.0, DWRITE_OVERHANG_METRICS::default(), 0.0, 0.0, 0.0);
    assert_eq!(bounds.width, 1.0);
    assert_eq!(bounds.height, 1.0);
}
