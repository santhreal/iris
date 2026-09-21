use super::Overlay;

// WHY: the committed-selection handle layout is a fixed contract:
// 4 corners then 4 edges, each centered on the point it drags. A
// reordered or off-center handle breaks resize hit-testing. Not
// covered: the GPU paint of the handles.
#[test]
fn handles_are_corners_then_edges() {
    let h = Overlay::handles(10.0, 20.0, 100.0, 50.0);
    // Corners NW NE SW SE.
    assert_eq!(h[0], (10.0, 20.0));
    assert_eq!(h[1], (110.0, 20.0));
    assert_eq!(h[2], (10.0, 70.0));
    assert_eq!(h[3], (110.0, 70.0));
    // Edges N S W E, centered on the midpoint.
    assert_eq!(h[4], (60.0, 20.0));
    assert_eq!(h[5], (60.0, 70.0));
    assert_eq!(h[6], (10.0, 45.0));
    assert_eq!(h[7], (110.0, 45.0));
}
