use crate::color::srgb_u8_to_oklab;
use anyhow::{Context, Result};
use image::imageops::FilterType;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Cluster {
    pub oklab: [f32; 3],
    pub weight: f32,
}

/// Below this Oklch chroma, a cluster's hue is noise, not signal -- shared
/// by `ImageStats::chromatic_mean_c` and the hue-matching/fallback logic in
/// `palette_gen` so "is this cluster chromatic" means the same thing
/// everywhere.
pub const CHROMATIC_THRESHOLD: f32 = 0.02;

pub struct ImageStats {
    pub mean_l: f32,
    /// Weighted mean chroma across *all* clusters, including near-black/gray
    /// ones. Dominated by whatever covers the most pixels -- for a mostly
    /// dark/desaturated wallpaper with a small vivid accent, this is close
    /// to zero and a poor reference for "how vivid should colors be".
    pub mean_c: f32,
    /// Weighted mean chroma across only the chromatic clusters (chroma >=
    /// `CHROMATIC_THRESHOLD`), falling back to `mean_c` if the image has no
    /// chromatic content at all. This is what `chroma_clamp_factor` is
    /// actually applied to -- it answers "how vivid are the colors that
    /// exist", not "how vivid is the image on average", so a small accent
    /// region isn't crushed just because most of the image is dark/gray.
    pub chromatic_mean_c: f32,
    pub is_dark: bool,
}

pub fn load_and_sample(path: &Path, max_dim: u32) -> Result<Vec<[f32; 3]>> {
    let img = image::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    let scale = max_dim as f32 / w.max(h) as f32;
    let resized = if scale < 1.0 {
        image::imageops::resize(
            &img,
            (w as f32 * scale).round().max(1.0) as u32,
            (h as f32 * scale).round().max(1.0) as u32,
            FilterType::Triangle,
        )
    } else {
        img
    };
    Ok(resized
        .pixels()
        .map(|p| srgb_u8_to_oklab([p[0], p[1], p[2]]))
        .collect())
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    dl * dl + da * da + db * db
}

/// Deterministic farthest-point seeding: start from the first point, then
/// repeatedly add whichever remaining point is farthest from the nearest
/// already-chosen center. Gives a spread-out, reproducible init without
/// pulling in a `rand` dependency.
fn farthest_point_seed(points: &[[f32; 3]], k: usize) -> Vec<[f32; 3]> {
    let mut centers = vec![points[0]];
    let mut min_d2: Vec<f32> = points.iter().map(|p| dist2(*p, points[0])).collect();
    while centers.len() < k && centers.len() < points.len() {
        let (best_idx, _) = min_d2
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        let next = points[best_idx];
        centers.push(next);
        for (i, p) in points.iter().enumerate() {
            let d = dist2(*p, next);
            if d < min_d2[i] {
                min_d2[i] = d;
            }
        }
    }
    centers
}

pub fn kmeans_oklab(points: &[[f32; 3]], k: usize, max_iters: usize) -> Vec<Cluster> {
    assert!(!points.is_empty());
    let k = k.min(points.len());
    let mut centers = farthest_point_seed(points, k);
    let mut assignments = vec![0usize; points.len()];

    for _ in 0..max_iters {
        let mut changed = false;
        for (i, p) in points.iter().enumerate() {
            let mut best = 0;
            let mut best_d = f32::MAX;
            for (ci, c) in centers.iter().enumerate() {
                let d = dist2(*p, *c);
                if d < best_d {
                    best_d = d;
                    best = ci;
                }
            }
            if assignments[i] != best {
                assignments[i] = best;
                changed = true;
            }
        }

        let mut sums = vec![[0f32; 3]; centers.len()];
        let mut counts = vec![0u32; centers.len()];
        for (i, p) in points.iter().enumerate() {
            let c = assignments[i];
            sums[c][0] += p[0];
            sums[c][1] += p[1];
            sums[c][2] += p[2];
            counts[c] += 1;
        }
        for (ci, center) in centers.iter_mut().enumerate() {
            if counts[ci] > 0 {
                *center = [
                    sums[ci][0] / counts[ci] as f32,
                    sums[ci][1] / counts[ci] as f32,
                    sums[ci][2] / counts[ci] as f32,
                ];
            }
        }

        if !changed {
            break;
        }
    }

    let mut counts = vec![0u32; centers.len()];
    for &a in &assignments {
        counts[a] += 1;
    }
    let total = points.len() as f32;
    centers
        .into_iter()
        .zip(counts)
        .filter(|(_, count)| *count > 0)
        .map(|(oklab, count)| Cluster {
            oklab,
            weight: count as f32 / total,
        })
        .collect()
}

pub fn image_stats(clusters: &[Cluster]) -> ImageStats {
    use crate::color::oklab_to_oklch;
    let mut mean_l = 0.0;
    let mut mean_c = 0.0;
    let mut chromatic_weight = 0.0;
    let mut chromatic_c_sum = 0.0;
    for c in clusters {
        let lch = oklab_to_oklch(c.oklab);
        mean_l += lch[0] * c.weight;
        mean_c += lch[1] * c.weight;
        if lch[1] >= CHROMATIC_THRESHOLD {
            chromatic_weight += c.weight;
            chromatic_c_sum += lch[1] * c.weight;
        }
    }
    let chromatic_mean_c = if chromatic_weight > 0.0 {
        chromatic_c_sum / chromatic_weight
    } else {
        mean_c
    };
    ImageStats {
        mean_l,
        mean_c,
        chromatic_mean_c,
        is_dark: mean_l < 0.55,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kmeans_separates_two_obvious_blobs() {
        let mut points = vec![];
        for _ in 0..50 {
            points.push([0.1, 0.0, 0.0]);
        }
        for _ in 0..50 {
            points.push([0.9, 0.0, 0.0]);
        }
        let clusters = kmeans_oklab(&points, 2, 20);
        assert_eq!(clusters.len(), 2);
        let mut ls: Vec<f32> = clusters.iter().map(|c| c.oklab[0]).collect();
        ls.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((ls[0] - 0.1).abs() < 1e-3);
        assert!((ls[1] - 0.9).abs() < 1e-3);
    }

    #[test]
    fn cluster_weights_sum_to_one() {
        let points = vec![[0.1, 0.0, 0.0], [0.5, 0.0, 0.0], [0.9, 0.0, 0.0]];
        let clusters = kmeans_oklab(&points, 3, 20);
        let total: f32 = clusters.iter().map(|c| c.weight).sum();
        assert!((total - 1.0).abs() < 1e-4);
    }

    #[test]
    fn chromatic_mean_c_ignores_dominant_achromatic_cluster() {
        use crate::color::oklch_to_oklab;
        // Mimics a mostly-black wallpaper with a small vivid accent: the
        // huge near-zero-chroma cluster should not drag chromatic_mean_c
        // down the way it drags plain mean_c down.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.1, 0.0, 0.0]),
                weight: 0.9,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.1, 45.0]),
                weight: 0.01,
            },
        ];
        let stats = image_stats(&clusters);
        assert!(stats.mean_c < 0.01);
        assert!((stats.chromatic_mean_c - 0.1).abs() < 1e-4);
    }

    #[test]
    fn chromatic_mean_c_falls_back_to_mean_c_when_fully_achromatic() {
        let clusters = vec![Cluster {
            oklab: [0.5, 0.0, 0.0],
            weight: 1.0,
        }];
        let stats = image_stats(&clusters);
        assert_eq!(stats.chromatic_mean_c, stats.mean_c);
    }
}
