//! User preference is separate from the definition actually authorized upstream.
use atv_core::hls::Variant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Auto,
    Sd,
    Hd,
    Fhd,
    Shd,
}
impl Quality {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "sd" => Some(Self::Sd),
            "hd" => Some(Self::Hd),
            "fhd" => Some(Self::Fhd),
            "shd" => Some(Self::Shd),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Sd => "sd",
            Self::Hd => "hd",
            Self::Fhd => "fhd",
            Self::Shd => "shd",
        }
    }
    pub fn request(self) -> &'static str {
        if self == Self::Auto { "hd" } else { self.id() }
    }
    pub fn choose(self, variants: &[Variant]) -> Option<&Variant> {
        self.choose_with_height(variants, None)
    }
    pub fn choose_with_height(
        self,
        variants: &[Variant],
        authorized_height: Option<u32>,
    ) -> Option<&Variant> {
        let cap = match self {
            Self::Sd => 576,
            Self::Auto | Self::Hd | Self::Shd => 720,
            Self::Fhd => 1080,
        };
        let cap = authorized_height.map_or(cap, |height| {
            if self == Self::Auto {
                cap.min(height)
            } else {
                height
            }
        });
        variants
            .iter()
            .filter(|v| v.resolution.is_none_or(|(_, h)| h <= cap))
            .max_by_key(|v| v.bandwidth)
            .or_else(|| variants.iter().min_by_key(|v| v.bandwidth))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requested_quality_changes_variant_selection_without_inventing_resolution() {
        let variants = [540, 720, 1080, 2160].map(|h| Variant {
            url: "https://fixture.invalid/live.m3u8".parse().unwrap(),
            bandwidth: u64::from(h),
            resolution: Some((h * 16 / 9, h)),
            codecs: None,
            audio: None,
        });
        assert_eq!(
            Quality::Sd.choose(&variants).unwrap().resolution.unwrap().1,
            540
        );
        assert_eq!(
            Quality::Auto
                .choose(&variants)
                .unwrap()
                .resolution
                .unwrap()
                .1,
            720
        );
        assert_eq!(
            Quality::Fhd
                .choose(&variants)
                .unwrap()
                .resolution
                .unwrap()
                .1,
            1080
        );
        assert_eq!(
            Quality::Shd
                .choose(&variants)
                .unwrap()
                .resolution
                .unwrap()
                .1,
            720
        );
        assert_eq!(
            Quality::Shd
                .choose_with_height(&variants, Some(1080))
                .unwrap()
                .resolution
                .unwrap()
                .1,
            1080
        );
        assert!(Quality::parse("4k-arbitrary").is_none());
    }
}
