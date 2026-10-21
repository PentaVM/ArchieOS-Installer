//! Box arithmetic done on the host: torchvision's `roi_align` and `nms`, to the same rules.

/// `torchvision.ops.roi_align(aligned=True, spatial_scale=1, sampling_ratio=2)` for one box
/// over a `[C, H, W]` map in row-major `values`. `box_xyxy` is in feature-map cells. Returns
/// `[C, out, out]`, channel-major, as `.flatten(1)` lays it out.
pub fn roi_align(
    values: &[f32],
    channels: usize,
    height: usize,
    width: usize,
    box_xyxy: [f32; 4],
    out: usize,
) -> Vec<f32> {
    const SAMPLING: usize = 2;
    let offset = 0.5f32;
    let start_w = box_xyxy[0] - offset;
    let start_h = box_xyxy[1] - offset;
    let end_w = box_xyxy[2] - offset;
    let end_h = box_xyxy[3] - offset;
    let bin_w = (end_w - start_w) / out as f32;
    let bin_h = (end_h - start_h) / out as f32;
    let count = (SAMPLING * SAMPLING) as f32;

    // The sample points and their bilinear weights do not depend on the channel.
    let mut samples: Vec<Vec<(usize, f32)>> = Vec::with_capacity(out * out);
    for ph in 0..out {
        for pw in 0..out {
            let mut taps = Vec::with_capacity(16);
            for iy in 0..SAMPLING {
                let y = start_h + ph as f32 * bin_h + (iy as f32 + 0.5) * bin_h / SAMPLING as f32;
                for ix in 0..SAMPLING {
                    let x =
                        start_w + pw as f32 * bin_w + (ix as f32 + 0.5) * bin_w / SAMPLING as f32;
                    bilinear_taps(height, width, y, x, &mut taps);
                }
            }
            samples.push(taps);
        }
    }
    let plane = height * width;
    let mut result = vec![0f32; channels * out * out];
    for channel in 0..channels {
        let map = &values[channel * plane..(channel + 1) * plane];
        for (bin, taps) in samples.iter().enumerate() {
            let total: f32 = taps
                .iter()
                .map(|&(index, weight)| weight * map[index])
                .sum();
            result[channel * out * out + bin] = total / count;
        }
    }
    result
}

/// [`roi_align`] as a matrix: `roi_align` is linear in the map, so the pooled values of
/// `boxes` over a `[C, H, W]` map are this `[boxes * out * out, H * W]` matrix times the map
/// laid out `[H * W, C]`. Row `box * out * out + bin` holds the bin's bilinear weights
/// divided by the sample count. The products sum in a different order than the loop in
/// `roi_align`, so the two agree to rounding, not to the bit.
pub fn roi_align_matrix(height: usize, width: usize, boxes: &[[f32; 4]], out: usize) -> Vec<f32> {
    const SAMPLING: usize = 2;
    let count = (SAMPLING * SAMPLING) as f32;
    let plane = height * width;
    let mut matrix = vec![0f32; boxes.len() * out * out * plane];
    let mut taps = Vec::with_capacity(16);
    for (index, box_xyxy) in boxes.iter().enumerate() {
        let offset = 0.5f32;
        let start_w = box_xyxy[0] - offset;
        let start_h = box_xyxy[1] - offset;
        let end_w = box_xyxy[2] - offset;
        let end_h = box_xyxy[3] - offset;
        let bin_w = (end_w - start_w) / out as f32;
        let bin_h = (end_h - start_h) / out as f32;
        for ph in 0..out {
            for pw in 0..out {
                taps.clear();
                for iy in 0..SAMPLING {
                    let y =
                        start_h + ph as f32 * bin_h + (iy as f32 + 0.5) * bin_h / SAMPLING as f32;
                    for ix in 0..SAMPLING {
                        let x = start_w
                            + pw as f32 * bin_w
                            + (ix as f32 + 0.5) * bin_w / SAMPLING as f32;
                        bilinear_taps(height, width, y, x, &mut taps);
                    }
                }
                let row = (index * out * out + ph * out + pw) * plane;
                for &(cell, weight) in &taps {
                    matrix[row + cell] += weight / count;
                }
            }
        }
    }
    matrix
}

fn bilinear_taps(height: usize, width: usize, y: f32, x: f32, taps: &mut Vec<(usize, f32)>) {
    if y < -1.0 || y > height as f32 || x < -1.0 || x > width as f32 {
        return;
    }
    let mut y = y.max(0.0);
    let mut x = x.max(0.0);
    let mut y_low = y as usize;
    let mut x_low = x as usize;
    let y_high;
    let x_high;
    if y_low >= height - 1 {
        y_low = height - 1;
        y_high = y_low;
        y = y_low as f32;
    } else {
        y_high = y_low + 1;
    }
    if x_low >= width - 1 {
        x_low = width - 1;
        x_high = x_low;
        x = x_low as f32;
    } else {
        x_high = x_low + 1;
    }
    let ly = y - y_low as f32;
    let lx = x - x_low as f32;
    let hy = 1.0 - ly;
    let hx = 1.0 - lx;
    taps.push((y_low * width + x_low, hy * hx));
    taps.push((y_low * width + x_high, hy * lx));
    taps.push((y_high * width + x_low, ly * hx));
    taps.push((y_high * width + x_high, ly * lx));
}

pub fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let union = area(a) + area(b) - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// `torchvision.ops.nms`: indices into `boxes` kept, highest score first; a box is dropped
/// when its IoU with a kept one is above `threshold`.
pub fn nms(boxes: &[[f32; 4]], scores: &[f32], threshold: f32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let mut kept: Vec<usize> = Vec::new();
    for index in order {
        if kept
            .iter()
            .all(|&other| iou(boxes[index], boxes[other]) <= threshold)
        {
            kept.push(index);
        }
    }
    kept
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_matrix_pools_what_roi_align_pools() {
        let (channels, height, width) = (5, 7, 9);
        let map: Vec<f32> = (0..channels * height * width)
            .map(|i| ((i * 2654435761usize) % 1000) as f32 / 250.0 - 2.0)
            .collect();
        let boxes = [
            [0.3f32, 1.2, 4.8, 6.1],
            [-1.5, -0.5, 2.0, 3.0],
            [6.0, 4.0, 9.5, 7.5],
            [2.0, 2.0, 2.0, 2.0],
        ];
        let out = 3;
        let matrix = roi_align_matrix(height, width, &boxes, out);
        let plane = height * width;
        for (index, b) in boxes.iter().enumerate() {
            let expected = roi_align(&map, channels, height, width, *b, out);
            for channel in 0..channels {
                for bin in 0..out * out {
                    let row = &matrix[(index * out * out + bin) * plane..][..plane];
                    let got: f32 = row
                        .iter()
                        .zip(&map[channel * plane..(channel + 1) * plane])
                        .map(|(w, v)| w * v)
                        .sum();
                    let want = expected[channel * out * out + bin];
                    assert!(
                        (got - want).abs() <= 1e-5,
                        "box {index} channel {channel} bin {bin}: {got} vs {want}"
                    );
                }
            }
        }
    }

    use super::*;

    #[test]
    fn a_constant_map_pools_to_its_constant() {
        let map = vec![3.0f32; 2 * 4 * 4];
        let pooled = roi_align(&map, 2, 4, 4, [0.5, 0.5, 3.5, 3.5], 3);
        assert!(pooled.iter().all(|&v| (v - 3.0).abs() < 1e-6));
    }

    #[test]
    fn overlapping_boxes_keep_the_stronger() {
        let boxes = [
            [0.0, 0.0, 1.0, 1.0],
            [0.05, 0.0, 1.0, 1.0],
            [2.0, 2.0, 3.0, 3.0],
        ];
        assert_eq!(nms(&boxes, &[0.5, 0.9, 0.1], 0.5), vec![1, 2]);
    }
}
