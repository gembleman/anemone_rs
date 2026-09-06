use super::*;

#[test]
fn client_point_packs_into_the_low_and_high_words() {
    assert_eq!(pack_point(POINT { x: 0, y: 0 }), 0);
    assert_eq!(pack_point(POINT { x: 3, y: 7 }), 0x0007_0003);
}

/// 목록의 왼쪽/위쪽 바깥을 누르면 client 좌표가 음수가 된다. `POINTS`는 16비트
/// 부호 있는 값이라, 상위 비트를 잘라내야 Windows가 같은 좌표로 되읽는다.
#[test]
fn negative_client_coordinates_keep_their_sign_in_16_bits() {
    let packed = pack_point(POINT { x: -5, y: -1 }) as u32;
    assert_eq!(packed & 0xffff, 0xfffb);
    assert_eq!((packed >> 16) & 0xffff, 0xffff);
}
