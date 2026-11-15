use shakmaty::Color;

pub fn color_bits(color: Color) -> u8 {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

pub fn color_from_bits(bits: u8) -> Option<Color> {
    match bits {
        0 => Some(Color::White),
        1 => Some(Color::Black),
        _ => None,
    }
}
