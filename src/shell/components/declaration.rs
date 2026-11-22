//! A `style` attribute's value, always as a declaration. A bare `7.5%` is
//! discarded by the browser, and whatever it placed renders at the origin
//! or flat while the numbers beside it are right.

pub fn left(percent: f64) -> String {
    format!("left:{percent:.2}%")
}

pub fn top(percent: f64) -> String {
    format!("top:{percent:.2}%")
}

pub fn span(left: f64, width: f64) -> String {
    format!("left:{left:.2}%;width:{width:.2}%")
}

/// Anchored on the side with room: from the left edge in the first three
/// fifths, from the right after, so what opens beside a point near the end
/// does not run off the card.
pub fn beside(percent: f64, margin: i32) -> String {
    match percent > 60.0 {
        true => format!("right:{:.2}%;margin-right:{margin}px", 100.0 - percent),
        false => format!("left:{percent:.2}%;margin-left:{margin}px"),
    }
}

pub fn cells(count: usize, gap: u32) -> String {
    format!("--cells:{count};--cells-gap:{gap}px")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_is_a_declaration() {
        for written in [
            left(7.5),
            top(7.5),
            span(1.0, 2.0),
            beside(70.0, -14),
            cells(30, 3),
        ] {
            assert!(written.contains(':'), "{written}");
            assert!(!written.ends_with(';'), "{written}");
        }
        assert_eq!(beside(70.0, -14), "right:30.00%;margin-right:-14px");
    }
}
