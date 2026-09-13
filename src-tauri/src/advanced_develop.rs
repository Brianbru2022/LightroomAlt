//! Milestone 15 advanced photographic Develop stages.
//!
//! The module owns validated recipe data and deterministic pixel algorithms.
//! Lens profiles are a deliberately small, pinned Lensfun-data subset; profile
//! identification remains separate from correction rendering.

use image::{imageops, Rgb, RgbImage};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub(crate) const LENSFUN_DATA_REVISION: &str = "12f5976ce30c024f98c420835125b9676ac07811";
pub(crate) const LENSFUN_DATA_LICENCE: &str = "CC-BY-SA-3.0";

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurvePoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

fn identity_points() -> Vec<CurvePoint> {
    vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }]
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ToneCurves {
    pub(crate) master: Vec<CurvePoint>,
    pub(crate) red: Vec<CurvePoint>,
    pub(crate) green: Vec<CurvePoint>,
    pub(crate) blue: Vec<CurvePoint>,
}

impl Default for ToneCurves {
    fn default() -> Self {
        Self {
            master: identity_points(),
            red: identity_points(),
            green: identity_points(),
            blue: identity_points(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct HslBand {
    pub(crate) hue: f32,
    pub(crate) saturation: f32,
    pub(crate) luminance: f32,
}

fn default_hsl_bands() -> Vec<HslBand> {
    vec![HslBand::default(); 8]
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ColourMixer {
    pub(crate) bands: Vec<HslBand>,
}

impl Default for ColourMixer {
    fn default() -> Self {
        Self {
            bands: default_hsl_bands(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct GradeWheel {
    pub(crate) hue: f32,
    pub(crate) saturation: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ColourGrading {
    pub(crate) shadows: GradeWheel,
    pub(crate) midtones: GradeWheel,
    pub(crate) highlights: GradeWheel,
    pub(crate) balance: f32,
    pub(crate) blending: f32,
}

impl Default for ColourGrading {
    fn default() -> Self {
        Self {
            shadows: GradeWheel::default(),
            midtones: GradeWheel::default(),
            highlights: GradeWheel::default(),
            balance: 0.0,
            blending: 50.0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct DetailSettings {
    pub(crate) sharpen_amount: f32,
    pub(crate) sharpen_radius: f32,
    pub(crate) sharpen_detail: f32,
    pub(crate) sharpen_masking: f32,
    pub(crate) luminance_nr: f32,
    pub(crate) luminance_detail: f32,
    pub(crate) luminance_contrast: f32,
    pub(crate) colour_nr: f32,
    pub(crate) colour_detail: f32,
    pub(crate) colour_smoothness: f32,
}

impl Default for DetailSettings {
    fn default() -> Self {
        Self {
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            luminance_nr: 0.0,
            luminance_detail: 50.0,
            luminance_contrast: 0.0,
            colour_nr: 0.0,
            colour_detail: 50.0,
            colour_smoothness: 50.0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LensCorrections {
    pub(crate) enabled: bool,
    pub(crate) profile_mode: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) profile_revision: Option<String>,
    pub(crate) profile_amount: f32,
    pub(crate) manual_distortion: f32,
    pub(crate) ca_red: f32,
    pub(crate) ca_blue: f32,
    pub(crate) vignette_amount: f32,
    pub(crate) vignette_midpoint: f32,
    pub(crate) constrain_crop: bool,
}

impl Default for LensCorrections {
    fn default() -> Self {
        Self {
            enabled: false,
            profile_mode: "off".into(),
            profile_id: None,
            profile_revision: None,
            profile_amount: 100.0,
            manual_distortion: 0.0,
            ca_red: 0.0,
            ca_blue: 0.0,
            vignette_amount: 0.0,
            vignette_midpoint: 50.0,
            constrain_crop: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AdvancedDevelopSettings {
    pub(crate) curves: ToneCurves,
    pub(crate) colour_mixer: ColourMixer,
    pub(crate) colour_grading: ColourGrading,
    pub(crate) detail: DetailSettings,
    pub(crate) lens: LensCorrections,
}

fn finite_range(value: f32, minimum: f32, maximum: f32) -> bool {
    value.is_finite() && (minimum..=maximum).contains(&value)
}

fn validate_curve(points: &mut [CurvePoint]) -> Result<(), String> {
    if points.len() < 2 || points.len() > 16 {
        return Err("A tone curve must contain 2 to 16 points.".into());
    }
    points.sort_by(|left, right| left.x.total_cmp(&right.x));
    if points.first().is_none_or(|point| point.x != 0.0)
        || points.last().is_none_or(|point| point.x != 1.0)
        || points
            .iter()
            .any(|point| !finite_range(point.x, 0.0, 1.0) || !finite_range(point.y, 0.0, 1.0))
        || points.windows(2).any(|pair| pair[1].x - pair[0].x < 0.005)
    {
        return Err(
            "Tone-curve points must be finite, ordered, unique, and retain 0/1 endpoints.".into(),
        );
    }
    Ok(())
}

impl AdvancedDevelopSettings {
    pub(crate) fn validate(mut self) -> Result<Self, String> {
        for curve in [
            &mut self.curves.master,
            &mut self.curves.red,
            &mut self.curves.green,
            &mut self.curves.blue,
        ] {
            validate_curve(curve)?;
        }
        if self.colour_mixer.bands.len() != 8
            || self.colour_mixer.bands.iter().any(|band| {
                !finite_range(band.hue, -100.0, 100.0)
                    || !finite_range(band.saturation, -100.0, 100.0)
                    || !finite_range(band.luminance, -100.0, 100.0)
            })
        {
            return Err("The Colour Mixer must contain eight valid hue bands.".into());
        }
        for wheel in [
            self.colour_grading.shadows,
            self.colour_grading.midtones,
            self.colour_grading.highlights,
        ] {
            if !finite_range(wheel.hue, 0.0, 360.0) || !finite_range(wheel.saturation, 0.0, 100.0) {
                return Err("Colour-grading wheel values are invalid.".into());
            }
        }
        if !finite_range(self.colour_grading.balance, -100.0, 100.0)
            || !finite_range(self.colour_grading.blending, 0.0, 100.0)
        {
            return Err("Colour-grading balance or blending is invalid.".into());
        }
        let detail = self.detail;
        if !finite_range(detail.sharpen_amount, 0.0, 150.0)
            || !finite_range(detail.sharpen_radius, 0.5, 3.0)
            || !finite_range(detail.sharpen_detail, 0.0, 100.0)
            || !finite_range(detail.sharpen_masking, 0.0, 100.0)
            || [
                detail.luminance_nr,
                detail.luminance_detail,
                detail.luminance_contrast,
                detail.colour_nr,
                detail.colour_detail,
                detail.colour_smoothness,
            ]
            .iter()
            .any(|value| !finite_range(*value, 0.0, 100.0))
        {
            return Err("One or more Detail controls are outside their supported range.".into());
        }
        let lens = &self.lens;
        if !matches!(lens.profile_mode.as_str(), "off" | "auto" | "manual")
            || !finite_range(lens.profile_amount, 0.0, 100.0)
            || !finite_range(lens.manual_distortion, -100.0, 100.0)
            || !finite_range(lens.ca_red, -100.0, 100.0)
            || !finite_range(lens.ca_blue, -100.0, 100.0)
            || !finite_range(lens.vignette_amount, -100.0, 100.0)
            || !finite_range(lens.vignette_midpoint, 0.0, 100.0)
        {
            return Err("One or more Lens Correction controls are invalid.".into());
        }
        if lens.enabled && lens.profile_mode != "off" {
            let id = lens.profile_id.as_deref().unwrap_or_default();
            let revision = lens.profile_revision.as_deref().unwrap_or_default();
            if id.is_empty()
                || id.len() > 128
                || revision.is_empty()
                || revision.len() > 80
                || !id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
                || !revision
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                return Err("The selected lens profile identity or revision is malformed.".into());
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LensProfile {
    pub(crate) id: &'static str,
    pub(crate) camera_contains: &'static str,
    pub(crate) lens: &'static str,
    pub(crate) focal_length: f32,
    pub(crate) aperture: f32,
    pub(crate) distortion: [f32; 3],
    pub(crate) ca_scale: [f32; 2],
    pub(crate) vignette: [f32; 3],
    pub(crate) source_file: &'static str,
}

// Exact Lensfun calibration rows, reduced to two deliberately strict matches.
pub(crate) const LENS_PROFILES: &[LensProfile] = &[
    LensProfile {
        id: "lensfun-canon-650d-ef50-f18-aps-c",
        camera_contains: "Canon EOS 650D",
        lens: "Canon EF 50mm f/1.8",
        focal_length: 50.0,
        aperture: 2.8,
        distortion: [-0.00149, 0.0, 0.0],
        ca_scale: [1.0000033, 0.9999942],
        vignette: [-0.1030, -0.0537, 0.0306],
        source_file: "slr-canon.xml",
    },
    LensProfile {
        id: "lensfun-nikon-d750-af50-f18d",
        camera_contains: "Nikon D750",
        lens: "Nikon AF Nikkor 50mm f/1.8D",
        focal_length: 50.0,
        aperture: 2.5,
        distortion: [0.00139, -0.00804, 0.00877],
        ca_scale: [1.0000537, 0.9998589],
        vignette: [-0.0795, -0.6086, 0.3210],
        source_file: "slr-nikon.xml",
    },
];

pub(crate) fn profile(id: &str) -> Option<&'static LensProfile> {
    LENS_PROFILES.iter().find(|candidate| candidate.id == id)
}

pub(crate) fn match_profile(
    camera: Option<&str>,
    lens: Option<&str>,
    focal_length: Option<f32>,
    aperture: Option<f32>,
) -> Option<&'static LensProfile> {
    let camera = camera?.trim();
    let lens = lens?.trim();
    LENS_PROFILES.iter().find(|candidate| {
        camera.eq_ignore_ascii_case(candidate.camera_contains)
            && lens.eq_ignore_ascii_case(candidate.lens)
            && focal_length.is_some_and(|value| (value - candidate.focal_length).abs() <= 0.25)
            && aperture.is_some_and(|value| (value - candidate.aperture).abs() <= 0.2)
    })
}

fn curve_value(points: &[CurvePoint], x: f32) -> f32 {
    if points.len() == 2 && points[0].x == points[0].y && points[1].x == points[1].y {
        return x;
    }
    let segment = points
        .windows(2)
        .position(|pair| x <= pair[1].x)
        .unwrap_or(points.len() - 2);
    let slopes = points
        .windows(2)
        .map(|pair| (pair[1].y - pair[0].y) / (pair[1].x - pair[0].x))
        .collect::<Vec<_>>();
    let tangent = |index: usize| {
        if index == 0 {
            slopes[0]
        } else if index >= points.len() - 1 {
            slopes[slopes.len() - 1]
        } else if slopes[index - 1] * slopes[index] <= 0.0 {
            0.0
        } else {
            2.0 * slopes[index - 1] * slopes[index] / (slopes[index - 1] + slopes[index])
        }
    };
    let left = points[segment];
    let right = points[segment + 1];
    let width = right.x - left.x;
    let t = ((x - left.x) / width).clamp(0.0, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    ((2.0 * t3 - 3.0 * t2 + 1.0) * left.y
        + (t3 - 2.0 * t2 + t) * width * tangent(segment)
        + (-2.0 * t3 + 3.0 * t2) * right.y
        + (t3 - t2) * width * tangent(segment + 1))
    .clamp(0.0, 1.0)
}

fn rgb_to_hsl(rgb: [f32; 3]) -> [f32; 3] {
    let max = rgb.into_iter().fold(0.0_f32, f32::max);
    let min = rgb.into_iter().fold(1.0_f32, f32::min);
    let lightness = (max + min) * 0.5;
    let delta = max - min;
    if delta < 0.000_001 {
        return [0.0, 0.0, lightness];
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs()).max(0.000_001);
    let hue = if max == rgb[0] {
        60.0 * (((rgb[1] - rgb[2]) / delta) % 6.0)
    } else if max == rgb[1] {
        60.0 * ((rgb[2] - rgb[0]) / delta + 2.0)
    } else {
        60.0 * ((rgb[0] - rgb[1]) / delta + 4.0)
    };
    [hue.rem_euclid(360.0), saturation, lightness]
}

fn hsl_to_rgb(hsl: [f32; 3]) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * hsl[2] - 1.0).abs()) * hsl[1];
    let section = hsl[0].rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (section % 2.0 - 1.0).abs());
    let base = match section as i32 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let m = hsl[2] - chroma * 0.5;
    [base[0] + m, base[1] + m, base[2] + m]
}

fn circular_distance(left: f32, right: f32) -> f32 {
    let difference = (left - right).abs().rem_euclid(360.0);
    difference.min(360.0 - difference)
}

fn hue_rgb(hue: f32) -> [f32; 3] {
    hsl_to_rgb([hue, 1.0, 0.5])
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0).max(0.000_001)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn apply_tone_colour(image: &mut RgbImage, advanced: &AdvancedDevelopSettings) {
    let centres = [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 285.0, 330.0];
    image.as_mut().par_chunks_mut(3).for_each(|pixel| {
        let mut rgb = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let luminance = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        let target = curve_value(&advanced.curves.master, luminance);
        let ratio = if luminance > 0.000_001 {
            target / luminance
        } else {
            0.0
        };
        for value in &mut rgb {
            *value = (*value * ratio).clamp(0.0, 1.0);
        }
        rgb[0] = curve_value(&advanced.curves.red, rgb[0]);
        rgb[1] = curve_value(&advanced.curves.green, rgb[1]);
        rgb[2] = curve_value(&advanced.curves.blue, rgb[2]);
        let mut hsl = rgb_to_hsl(rgb);
        let mut total = 0.0;
        let mut h = 0.0;
        let mut s = 0.0;
        let mut l = 0.0;
        for (index, band) in advanced.colour_mixer.bands.iter().enumerate() {
            let weight = 1.0 - smoothstep(20.0, 55.0, circular_distance(hsl[0], centres[index]));
            total += weight;
            h += band.hue * weight;
            s += band.saturation * weight;
            l += band.luminance * weight;
        }
        if total > 0.0 {
            hsl[0] = (hsl[0] + h / total * 0.30).rem_euclid(360.0);
            hsl[1] = (hsl[1] * (1.0 + s / total / 100.0)).clamp(0.0, 1.0);
            let lum = l / total / 100.0;
            hsl[2] = if lum >= 0.0 {
                hsl[2] + (1.0 - hsl[2]) * lum * 0.5
            } else {
                hsl[2] * (1.0 + lum * 0.5)
            };
        }
        rgb = hsl_to_rgb(hsl);
        let balance = advanced.colour_grading.balance / 100.0 * 0.2;
        let overlap = 0.12 + advanced.colour_grading.blending / 100.0 * 0.28;
        let shadow = 1.0
            - smoothstep(
                (0.42 + balance - overlap).clamp(0.05, 0.85),
                (0.42 + balance + overlap).clamp(0.15, 0.95),
                hsl[2],
            );
        let highlight = smoothstep(
            (0.58 + balance - overlap).clamp(0.05, 0.85),
            (0.58 + balance + overlap).clamp(0.15, 0.95),
            hsl[2],
        );
        let midtone = (1.0 - shadow.max(highlight)).clamp(0.0, 1.0);
        for (wheel, weight) in [
            (advanced.colour_grading.shadows, shadow),
            (advanced.colour_grading.midtones, midtone),
            (advanced.colour_grading.highlights, highlight),
        ] {
            let amount = wheel.saturation / 100.0 * weight * 0.35;
            let tint = hue_rgb(wheel.hue);
            for channel in 0..3 {
                rgb[channel] =
                    (rgb[channel] * (1.0 - amount) + tint[channel] * amount).clamp(0.0, 1.0);
            }
        }
        for channel in 0..3 {
            pixel[channel] = (rgb[channel] * 255.0).round() as u8;
        }
    });
}

fn luminance(pixel: &Rgb<u8>) -> f32 {
    pixel[0] as f32 * 0.2126 + pixel[1] as f32 * 0.7152 + pixel[2] as f32 * 0.0722
}

pub(crate) fn apply_detail(image: &mut RgbImage, detail: DetailSettings) {
    if detail.luminance_nr > 0.0 {
        let original = image.clone();
        let blurred = imageops::blur(&original, 0.8 + detail.luminance_nr / 100.0 * 1.8);
        let strength = detail.luminance_nr / 100.0;
        let preserve = detail.luminance_detail / 100.0;
        image
            .as_mut()
            .par_chunks_mut(3)
            .enumerate()
            .for_each(|(index, pixel)| {
                let x = index as u32 % original.width();
                let y = index as u32 / original.width();
                let source = original.get_pixel(x, y);
                let smooth = blurred.get_pixel(x, y);
                let difference = (luminance(source) - luminance(smooth)).abs() / 255.0;
                let edge_guard =
                    smoothstep(0.015 + preserve * 0.04, 0.12 + preserve * 0.18, difference);
                let mix = strength * (1.0 - edge_guard);
                let source_luma = luminance(source);
                let target_luma = source_luma + (luminance(smooth) - source_luma) * mix;
                let contrast = detail.luminance_contrast / 100.0 * 0.12;
                let target_luma =
                    (target_luma + (source_luma - luminance(smooth)) * contrast).clamp(0.0, 255.0);
                let ratio = target_luma / source_luma.max(0.5);
                for channel in 0..3 {
                    pixel[channel] =
                        (source[channel] as f32 * ratio).clamp(0.0, 255.0).round() as u8;
                }
            });
    }
    if detail.colour_nr > 0.0 {
        let original = image.clone();
        let blurred = imageops::blur(&original, 0.7 + detail.colour_smoothness / 100.0 * 2.3);
        let amount = detail.colour_nr / 100.0;
        let preserve = detail.colour_detail / 100.0;
        image
            .as_mut()
            .par_chunks_mut(3)
            .enumerate()
            .for_each(|(index, pixel)| {
                let x = index as u32 % original.width();
                let y = index as u32 / original.width();
                let source = original.get_pixel(x, y);
                let smooth = blurred.get_pixel(x, y);
                let y0 = luminance(source);
                let edge = ((luminance(source) - luminance(smooth)).abs() / 255.0).clamp(0.0, 1.0);
                let mix = amount * (1.0 - edge * preserve).clamp(0.0, 1.0);
                for channel in 0..3 {
                    let source_chroma = source[channel] as f32 - y0;
                    let smooth_chroma = smooth[channel] as f32 - luminance(smooth);
                    pixel[channel] = (y0 + source_chroma + (smooth_chroma - source_chroma) * mix)
                        .clamp(0.0, 255.0)
                        .round() as u8;
                }
            });
    }
    if detail.sharpen_amount > 0.0 {
        let original = image.clone();
        let broad = imageops::blur(&original, detail.sharpen_radius);
        let fine = imageops::blur(&original, 0.55);
        let amount = detail.sharpen_amount / 100.0;
        let detail_mix = detail.sharpen_detail / 100.0;
        let threshold = detail.sharpen_masking / 100.0 * 0.18;
        image
            .as_mut()
            .par_chunks_mut(3)
            .enumerate()
            .for_each(|(index, pixel)| {
                let x = index as u32 % original.width();
                let y = index as u32 / original.width();
                let source = original.get_pixel(x, y);
                let edge = ((luminance(source) - luminance(broad.get_pixel(x, y))).abs() / 255.0)
                    .clamp(0.0, 1.0);
                let edge_mask = smoothstep(threshold, threshold + 0.08, edge);
                for channel in 0..3 {
                    let broad_detail =
                        source[channel] as f32 - broad.get_pixel(x, y)[channel] as f32;
                    let fine_detail = source[channel] as f32 - fine.get_pixel(x, y)[channel] as f32;
                    pixel[channel] = (source[channel] as f32
                        + amount
                            * edge_mask
                            * (broad_detail * (1.0 - detail_mix) + fine_detail * detail_mix * 0.7))
                        .clamp(0.0, 255.0)
                        .round() as u8;
                }
            });
    }
}

fn lens_parameters(settings: &LensCorrections) -> ([f32; 3], [f32; 2], [f32; 3]) {
    let mut distortion = [settings.manual_distortion / 100.0 * 0.18, 0.0, 0.0];
    let mut ca = [
        1.0 + settings.ca_red / 100.0 * 0.006,
        1.0 + settings.ca_blue / 100.0 * 0.006,
    ];
    let mut vignette = [settings.vignette_amount / 100.0 * -0.7, 0.0, 0.0];
    if settings.enabled && settings.profile_mode != "off" {
        if let Some(profile) = settings.profile_id.as_deref().and_then(profile) {
            let mix = settings.profile_amount / 100.0;
            for index in 0..3 {
                distortion[index] += profile.distortion[index] * mix;
                vignette[index] += profile.vignette[index] * mix;
            }
            ca[0] *= 1.0 + (profile.ca_scale[0] - 1.0) * mix;
            ca[1] *= 1.0 + (profile.ca_scale[1] - 1.0) * mix;
        }
    }
    (distortion, ca, vignette)
}

pub(crate) fn optics_active(settings: &LensCorrections) -> bool {
    settings.enabled
        && (settings.profile_mode != "off"
            || settings.manual_distortion.abs() > f32::EPSILON
            || settings.ca_red.abs() > f32::EPSILON
            || settings.ca_blue.abs() > f32::EPSILON
            || settings.vignette_amount.abs() > f32::EPSILON)
}

fn source_coordinate(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    settings: &LensCorrections,
    channel_scale: f32,
) -> (f32, f32) {
    let (distortion, _, _) = lens_parameters(settings);
    let aspect = width as f32 / height.max(1) as f32;
    let mut nx = (x as f32 / (width - 1).max(1) as f32 - 0.5) * 2.0;
    let mut ny = (y as f32 / (height - 1).max(1) as f32 - 0.5) * 2.0;
    nx *= aspect;
    let radius2 = nx * nx + ny * ny;
    let radial = 1.0
        + distortion[0] * radius2
        + distortion[1] * radius2 * radius2
        + distortion[2] * radius2 * radius2 * radius2;
    let crop = if settings.constrain_crop {
        1.0 / (1.0 + distortion.iter().map(|v| v.abs()).sum::<f32>() * 1.4)
    } else {
        1.0
    };
    nx *= radial * channel_scale * crop;
    ny *= radial * channel_scale * crop;
    (
        ((nx / aspect) * 0.5 + 0.5) * (width - 1) as f32,
        (ny * 0.5 + 0.5) * (height - 1) as f32,
    )
}

fn sample(image: &RgbImage, x: f32, y: f32, channel: usize) -> u8 {
    if x < 0.0 || y < 0.0 || x > (image.width() - 1) as f32 || y > (image.height() - 1) as f32 {
        return 0;
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let top = image.get_pixel(x0, y0)[channel] as f32 * (1.0 - fx)
        + image.get_pixel(x1, y0)[channel] as f32 * fx;
    let bottom = image.get_pixel(x0, y1)[channel] as f32 * (1.0 - fx)
        + image.get_pixel(x1, y1)[channel] as f32 * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

pub(crate) fn apply_optics(image: &RgbImage, settings: &LensCorrections) -> RgbImage {
    if !optics_active(settings) {
        return image.clone();
    }
    let width = image.width();
    let height = image.height();
    let (_, ca, vignette) = lens_parameters(settings);
    let pixels = (0..width as usize * height as usize)
        .into_par_iter()
        .flat_map_iter(|index| {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let green = source_coordinate(x, y, width, height, settings, 1.0);
            let red = source_coordinate(x, y, width, height, settings, ca[0]);
            let blue = source_coordinate(x, y, width, height, settings, ca[1]);
            let mut rgb = [
                sample(image, red.0, red.1, 0),
                sample(image, green.0, green.1, 1),
                sample(image, blue.0, blue.1, 2),
            ];
            let nx = (x as f32 / (width - 1).max(1) as f32 - 0.5) * 2.0;
            let ny = (y as f32 / (height - 1).max(1) as f32 - 0.5) * 2.0;
            let r2 = (nx * nx + ny * ny).min(2.0);
            let attenuation =
                (1.0 + vignette[0] * r2 + vignette[1] * r2 * r2 + vignette[2] * r2 * r2 * r2)
                    .max(0.2);
            let midpoint = 0.35 + settings.vignette_midpoint / 100.0 * 0.55;
            let correction = 1.0 + (1.0 / attenuation - 1.0) * smoothstep(midpoint, 1.4, r2.sqrt());
            for value in &mut rgb {
                *value = (*value as f32 * correction).round().clamp(0.0, 255.0) as u8;
            }
            rgb
        })
        .collect::<Vec<_>>();
    RgbImage::from_raw(width, height, pixels).expect("optical remap preserves dimensions")
}

fn sample_scalar(values: &[f32], width: u32, height: u32, x: f32, y: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x > (width - 1) as f32 || y > (height - 1) as f32 {
        return 0.0;
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |px: u32, py: u32| values[(py * width + px) as usize];
    (at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx) * (1.0 - fy)
        + (at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx) * fy
}

pub(crate) fn remap_mask(
    values: &[f32],
    width: u32,
    height: u32,
    settings: &LensCorrections,
) -> Vec<f32> {
    if !optics_active(settings) {
        return values.to_vec();
    }
    (0..width as usize * height as usize)
        .into_par_iter()
        .map(|index| {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let source = source_coordinate(x, y, width, height, settings, 1.0);
            sample_scalar(values, width, height, source.0, source.1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_validate_and_identity_is_exact() {
        let settings = AdvancedDevelopSettings::default().validate().unwrap();
        for value in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert_eq!(curve_value(&settings.curves.master, value), value);
        }
        let mut invalid = settings;
        invalid.curves.master = vec![CurvePoint { x: 0.2, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }];
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn strict_profile_match_never_fuzzily_substitutes() {
        assert!(match_profile(
            Some("Canon EOS 650D"),
            Some("Canon EF 50mm f/1.8"),
            Some(50.0),
            Some(2.8)
        )
        .is_some());
        assert!(match_profile(
            Some("Canon EOS 650D"),
            Some("Canon EF 50mm f/1.4"),
            Some(50.0),
            Some(2.8)
        )
        .is_none());
        assert!(match_profile(
            Some("Canon EOS 650D"),
            Some("Canon EF 50mm f/1.8"),
            None,
            Some(2.8)
        )
        .is_none());
    }

    #[test]
    fn missing_profile_identity_is_preserved_without_substitution() {
        let mut advanced = AdvancedDevelopSettings::default();
        advanced.lens.enabled = true;
        advanced.lens.profile_mode = "manual".into();
        advanced.lens.profile_id = Some("lensfun-profile-no-longer-installed".into());
        advanced.lens.profile_revision = Some("deadbeef".into());
        advanced.lens.manual_distortion = 20.0;
        let validated = advanced.validate().unwrap();
        assert!(profile(validated.lens.profile_id.as_deref().unwrap()).is_none());
        let grid = RgbImage::from_fn(25, 25, |x, y| {
            if x % 6 == 0 || y % 6 == 0 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            }
        });
        assert_ne!(apply_optics(&grid, &validated.lens), grid);

        let mut malformed = validated;
        malformed.lens.profile_id = Some("../profile".into());
        assert!(malformed.validate().is_err());
    }

    #[test]
    fn hsl_grading_detail_and_optics_have_distinct_effects() {
        let mut image = RgbImage::from_fn(32, 24, |x, y| {
            Rgb([(x * 7) as u8, (y * 9) as u8, ((x + y) * 4) as u8])
        });
        image.put_pixel(16, 12, Rgb([255, 0, 255]));
        let original = image.clone();
        let mut advanced = AdvancedDevelopSettings::default();
        advanced.colour_mixer.bands[0].saturation = 40.0;
        advanced.colour_grading.shadows = GradeWheel {
            hue: 220.0,
            saturation: 20.0,
        };
        apply_tone_colour(&mut image, &advanced);
        assert_ne!(image, original);
        let coloured = image.clone();
        advanced.detail.sharpen_amount = 80.0;
        apply_detail(&mut image, advanced.detail);
        assert_ne!(image, coloured);
        advanced.lens.enabled = true;
        advanced.lens.manual_distortion = 40.0;
        assert_ne!(apply_optics(&image, &advanced.lens), image);
    }

    #[test]
    fn curve_fixtures_cover_lift_crush_s_curve_and_channel_mapping() {
        let identity = identity_points();
        assert!((curve_value(&identity, 0.37) - 0.37).abs() < f32::EPSILON);
        let lifted = vec![
            CurvePoint { x: 0.0, y: 0.08 },
            CurvePoint { x: 0.3, y: 0.4 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        let crushed = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.2, y: 0.08 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        let s_curve = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.25, y: 0.18 },
            CurvePoint { x: 0.75, y: 0.84 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        assert!(curve_value(&lifted, 0.1) > 0.1);
        assert!(curve_value(&crushed, 0.1) < 0.1);
        assert!(curve_value(&s_curve, 0.2) < 0.2 && curve_value(&s_curve, 0.8) > 0.8);
        let mut image = RgbImage::from_pixel(1, 1, Rgb([100, 100, 100]));
        let mut advanced = AdvancedDevelopSettings::default();
        advanced.curves.red = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.7 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        apply_tone_colour(&mut image, &advanced);
        assert!(image.get_pixel(0, 0)[0] > image.get_pixel(0, 0)[1]);
    }

    #[test]
    fn hsl_overlap_wrap_and_grading_zones_are_smooth_and_targeted() {
        let mut red = RgbImage::from_pixel(1, 1, Rgb([220, 35, 35]));
        let mut blue = RgbImage::from_pixel(1, 1, Rgb([35, 35, 220]));
        let blue_before = blue.clone();
        let mut advanced = AdvancedDevelopSettings::default();
        advanced.colour_mixer.bands[0].saturation = -70.0;
        advanced.colour_mixer.bands[7].hue = 20.0;
        apply_tone_colour(&mut red, &advanced);
        apply_tone_colour(&mut blue, &advanced);
        assert_ne!(red.get_pixel(0, 0), &Rgb([220, 35, 35]));
        assert_eq!(blue, blue_before);
        let mut ramp = RgbImage::from_fn(64, 1, |x, _| Rgb([(x * 4) as u8; (3)]));
        advanced.colour_grading.shadows = GradeWheel {
            hue: 220.0,
            saturation: 35.0,
        };
        advanced.colour_grading.highlights = GradeWheel {
            hue: 45.0,
            saturation: 35.0,
        };
        apply_tone_colour(&mut ramp, &advanced);
        assert!(ramp.get_pixel(4, 0)[2] > ramp.get_pixel(4, 0)[0]);
        assert!(ramp.get_pixel(58, 0)[0] > ramp.get_pixel(58, 0)[2]);
    }

    #[test]
    fn noise_reduction_smooths_noise_while_edge_aware_sharpening_spares_flats() {
        let noisy = RgbImage::from_fn(48, 32, |x, y| {
            let base = if x < 24 { 50 } else { 190 };
            let n = ((x * 17 + y * 29) % 19) as i16 - 9;
            Rgb([(base + n) as u8, (base - n) as u8, (base + n / 2) as u8])
        });
        let variance = |image: &RgbImage| {
            image
                .pixels()
                .map(|pixel| (pixel[0] as f32 - 50.0).powi(2))
                .take(20 * 32)
                .sum::<f32>()
        };
        let mut denoised = noisy.clone();
        let settings = DetailSettings {
            luminance_nr: 60.0,
            colour_nr: 60.0,
            ..DetailSettings::default()
        };
        apply_detail(&mut denoised, settings);
        assert!(variance(&denoised) < variance(&noisy));
        let flat = RgbImage::from_pixel(40, 30, Rgb([120, 120, 120]));
        let mut sharpened = flat.clone();
        let sharpen = DetailSettings {
            sharpen_amount: 100.0,
            sharpen_masking: 80.0,
            ..DetailSettings::default()
        };
        apply_detail(&mut sharpened, sharpen);
        assert_eq!(flat, sharpened);
    }

    #[test]
    fn manual_barrel_pincushion_ca_and_vignette_are_deterministic() {
        let grid = RgbImage::from_fn(41, 41, |x, y| {
            if x % 8 == 0 || y % 8 == 0 {
                Rgb([255, 255, 255])
            } else {
                Rgb([20, 20, 20])
            }
        });
        let mut lens = LensCorrections {
            enabled: true,
            manual_distortion: 50.0,
            ..LensCorrections::default()
        };
        let barrel = apply_optics(&grid, &lens);
        lens.manual_distortion = -50.0;
        let pincushion = apply_optics(&grid, &lens);
        assert_ne!(barrel, pincushion);
        lens.manual_distortion = 0.0;
        lens.ca_red = 60.0;
        lens.ca_blue = -60.0;
        let ca = apply_optics(&grid, &lens);
        assert_ne!(ca, grid);
        lens.ca_red = 0.0;
        lens.ca_blue = 0.0;
        lens.vignette_amount = 50.0;
        let vignette = apply_optics(&grid, &lens);
        assert_eq!(vignette.get_pixel(20, 20), grid.get_pixel(20, 20));
        assert_ne!(vignette.get_pixel(1, 1), grid.get_pixel(1, 1));
        assert_eq!(vignette, apply_optics(&grid, &lens));
    }

    #[test]
    fn optical_mask_remap_matches_image_mapping() {
        let lens = LensCorrections {
            enabled: true,
            manual_distortion: 60.0,
            ..LensCorrections::default()
        };
        let image = RgbImage::from_fn(31, 31, |x, y| {
            if x == 15 && y == 15 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            }
        });
        let mask = image
            .pixels()
            .map(|pixel| pixel[0] as f32 / 255.0)
            .collect::<Vec<_>>();
        let remapped_image = apply_optics(&image, &lens);
        let remapped_mask = remap_mask(&mask, 31, 31, &lens);
        let brightest_image = remapped_image
            .pixels()
            .enumerate()
            .max_by_key(|(_, pixel)| pixel[0])
            .unwrap()
            .0;
        let brightest_mask = remapped_mask
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(brightest_image, brightest_mask);
    }

    #[test]
    #[ignore = "manual Milestone 15 stage benchmark; run scripts/benchmark-advanced-develop.ps1"]
    fn advanced_develop_performance_checkpoint() {
        use std::time::Instant;
        let fixture = |width, height| {
            RgbImage::from_fn(width, height, |x, y| {
                Rgb([
                    ((x * 13 + y * 3) % 256) as u8,
                    ((x * 5 + y * 11) % 256) as u8,
                    ((x * 7 + y * 17) % 256) as u8,
                ])
            })
        };
        let timed = |name: &str, mut run: Box<dyn FnMut()>| {
            let cold_started = Instant::now();
            run();
            let cold = cold_started.elapsed().as_secs_f64() * 1000.0;
            let warm_started = Instant::now();
            run();
            let warm = warm_started.elapsed().as_secs_f64() * 1000.0;
            println!("M15_STAGE {name}_cold_ms={cold:0.3} {name}_warm_ms={warm:0.3}");
        };
        let base = fixture(720, 480);
        let detail_region = fixture(1024, 768);

        let mut master_curve = AdvancedDevelopSettings::default();
        master_curve.curves.master = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.25, y: 0.20 },
            CurvePoint { x: 0.75, y: 0.82 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        timed(
            "master_curve_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_tone_colour(&mut image, &master_curve);
                std::hint::black_box(image);
            }),
        );

        let mut rgb_curves = AdvancedDevelopSettings::default();
        rgb_curves.curves.red = vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.54 },
            CurvePoint { x: 1.0, y: 1.0 },
        ];
        timed(
            "rgb_curves_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_tone_colour(&mut image, &rgb_curves);
                std::hint::black_box(image);
            }),
        );

        let mut hsl = AdvancedDevelopSettings::default();
        hsl.colour_mixer.bands[0].hue = 18.0;
        hsl.colour_mixer.bands[5].saturation = 28.0;
        timed(
            "hsl_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_tone_colour(&mut image, &hsl);
                std::hint::black_box(image);
            }),
        );

        let mut grading = AdvancedDevelopSettings::default();
        grading.colour_grading.shadows = GradeWheel {
            hue: 215.0,
            saturation: 24.0,
        };
        grading.colour_grading.highlights = GradeWheel {
            hue: 42.0,
            saturation: 18.0,
        };
        timed(
            "grading_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_tone_colour(&mut image, &grading);
                std::hint::black_box(image);
            }),
        );

        let sharpen = DetailSettings {
            sharpen_amount: 60.0,
            ..DetailSettings::default()
        };
        timed(
            "sharpen_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_detail(&mut image, sharpen);
                std::hint::black_box(image);
            }),
        );

        let luminance_nr = DetailSettings {
            luminance_nr: 35.0,
            ..DetailSettings::default()
        };
        timed(
            "luminance_nr_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_detail(&mut image, luminance_nr);
                std::hint::black_box(image);
            }),
        );

        let colour_nr = DetailSettings {
            colour_nr: 30.0,
            ..DetailSettings::default()
        };
        timed(
            "colour_nr_720x480",
            Box::new(|| {
                let mut image = base.clone();
                apply_detail(&mut image, colour_nr);
                std::hint::black_box(image);
            }),
        );

        let combined_detail = DetailSettings {
            sharpen_amount: 60.0,
            luminance_nr: 35.0,
            colour_nr: 30.0,
            ..DetailSettings::default()
        };
        timed(
            "detail_1024x768",
            Box::new(|| {
                let mut image = detail_region.clone();
                apply_detail(&mut image, combined_detail);
                std::hint::black_box(image);
            }),
        );

        let distortion = LensCorrections {
            enabled: true,
            manual_distortion: 28.0,
            ..LensCorrections::default()
        };
        let ca = LensCorrections {
            enabled: true,
            ca_red: 15.0,
            ca_blue: -12.0,
            ..LensCorrections::default()
        };
        let vignette = LensCorrections {
            enabled: true,
            vignette_amount: 25.0,
            ..LensCorrections::default()
        };
        for (name, lens) in [
            ("distortion_720x480", distortion.clone()),
            ("ca_720x480", ca.clone()),
            ("vignette_720x480", vignette.clone()),
        ] {
            let optics_base = base.clone();
            timed(
                name,
                Box::new(move || {
                    std::hint::black_box(apply_optics(&optics_base, &lens));
                }),
            );
        }

        let lens = LensCorrections {
            enabled: true,
            manual_distortion: 28.0,
            ca_red: 15.0,
            ca_blue: -12.0,
            vignette_amount: 25.0,
            ..LensCorrections::default()
        };
        let mut advanced = master_curve.clone();
        advanced.curves.red = rgb_curves.curves.red.clone();
        advanced.colour_mixer = hsl.colour_mixer.clone();
        advanced.colour_grading = grading.colour_grading.clone();
        let export = fixture(2400, 1600);
        timed(
            "full_advanced_2400x1600",
            Box::new(|| {
                let mut image = apply_optics(&export, &lens);
                apply_tone_colour(&mut image, &advanced);
                apply_detail(&mut image, combined_detail);
                std::hint::black_box(image);
            }),
        );
    }
}
