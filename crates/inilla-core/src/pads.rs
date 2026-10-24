//! Vanillish: **which controller is which**, from the ids every platform reports — the glyph
//! family a USB vendor/product id names. Android reads them off the `InputDevice`, Windows off
//! Bevy's `Gamepad` (gilrs, Windows.Gaming.Input); both report names the name test alone gets
//! wrong (a DualSense is "Sony Interactive Entertainment Wireless Controller" on Android and
//! "PS5 Controller" through gilrs), so the app reads `"{name} {family}"`.

/// The glyph family a vendor/product id names, the [`crate::pad::GlyphStyle::from_product`]
/// spelling of it. Empty when the vendor says nothing (the name still might: "… PlayStation
/// Edition").
pub fn family(vendor: u16, product: u16) -> &'static str {
    match (vendor, product) {
        // Sony: the DualSense and its Edge, the DualShock 4 (both revisions and the USB dongle).
        (0x054C, 0x0CE6 | 0x0DF2) => "DualSense",
        (0x054C, 0x05C4 | 0x09CC | 0x0BA0) => "DualShock 4",
        (0x054C, _) => "PlayStation",
        // Backbone One: the PlayStation Edition prints PlayStation symbols, the others Xbox
        // letters (SDL's `controller_list.h`, gamecontrollerdb).
        (0x358A, 0x0104 | 0x0304) => "PlayStation",
        (0x358A, _) => "Backbone",
        // Nintendo: Switch Pro, Joy-Cons, the Switch 2 Pro — all printed the Nintendo way.
        (0x057E, _) => "Nintendo Switch",
        (0x045E, _) => "Xbox",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::GlyphStyle;

    #[test]
    fn families_pick_the_glyphs() {
        let style =
            |v, p| GlyphStyle::from_product(&format!("Wireless Controller {}", family(v, p)));
        assert_eq!(style(0x054C, 0x0CE6), GlyphStyle::PlayStation);
        assert_eq!(style(0x054C, 0x0DF2), GlyphStyle::PlayStation);
        assert_eq!(style(0x054C, 0x09CC), GlyphStyle::PlayStation);
        assert_eq!(style(0x054C, 0x1234), GlyphStyle::PlayStation);
        assert_eq!(style(0x358A, 0x0304), GlyphStyle::PlayStation);
        assert_eq!(style(0x358A, 0x0104), GlyphStyle::PlayStation);
        assert_eq!(style(0x358A, 0x0201), GlyphStyle::Xbox);
        assert_eq!(style(0x057E, 0x2009), GlyphStyle::Nintendo);
        assert_eq!(style(0x057E, 0x2069), GlyphStyle::Nintendo);
        assert_eq!(style(0x045E, 0x0B13), GlyphStyle::Xbox);
        assert_eq!(family(0x1234, 0x5678), "");
        // An unknown id leaves it to the name.
        assert_eq!(
            GlyphStyle::from_product(&format!(
                "Backbone One - PlayStation Edition {}",
                family(0x358A, 0x9999)
            )),
            GlyphStyle::PlayStation
        );
    }

    #[test]
    fn gilrs_names_meet_their_family() {
        // gilrs names SDL-mapped pads after the mapping ("PS5 Controller"), which the name test
        // alone reads as Xbox; the family puts it right.
        let style = |name: &str, v, p| GlyphStyle::from_product(&format!("{name} {}", family(v, p)));
        assert_eq!(style("PS5 Controller", 0x054C, 0x0CE6), GlyphStyle::PlayStation);
        assert_eq!(style("PS4 Controller", 0x054C, 0x09CC), GlyphStyle::PlayStation);
        assert_eq!(
            style("Nintendo Switch Pro Controller", 0x057E, 0x2009),
            GlyphStyle::Nintendo
        );
        assert_eq!(style("Xbox Wireless Controller", 0x045E, 0x0B13), GlyphStyle::Xbox);
    }
}
