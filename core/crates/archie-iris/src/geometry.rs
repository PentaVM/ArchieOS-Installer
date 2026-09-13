use serde::{Deserialize, Serialize};
use thiserror::Error;

const SCALE: i64 = 1_000;
const MAX_COORDINATE: i32 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GeometryError {
    #[error("rectangle is empty or outside the coordinate bound")]
    InvalidRect,
    #[error("coordinate transform overflowed")]
    Overflow,
    #[error("affine transform is invalid")]
    InvalidTransform,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, GeometryError> {
        let rect = Self {
            x,
            y,
            width,
            height,
        };
        rect.validate()?;
        Ok(rect)
    }

    pub fn validate(&self) -> Result<(), GeometryError> {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.x.unsigned_abs() > MAX_COORDINATE as u32
            || self.y.unsigned_abs() > MAX_COORDINATE as u32
            || right.unsigned_abs() > MAX_COORDINATE as u64
            || bottom.unsigned_abs() > MAX_COORDINATE as u64
        {
            return Err(GeometryError::InvalidRect);
        }
        Ok(())
    }

    pub fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    pub fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    pub fn intersects(&self, other: &Self) -> bool {
        i64::from(self.x) < other.right()
            && self.right() > i64::from(other.x)
            && i64::from(self.y) < other.bottom()
            && self.bottom() > i64::from(other.y)
    }

    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then(|| Self {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }

    pub fn union(&self, other: &Self) -> Result<Self, GeometryError> {
        let left = i64::from(self.x).min(i64::from(other.x));
        let top = i64::from(self.y).min(i64::from(other.y));
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(
            i32::try_from(left).map_err(|_| GeometryError::Overflow)?,
            i32::try_from(top).map_err(|_| GeometryError::Overflow)?,
            u32::try_from(right - left).map_err(|_| GeometryError::Overflow)?,
            u32::try_from(bottom - top).map_err(|_| GeometryError::Overflow)?,
        )
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn iou_permille(&self, other: &Self) -> u16 {
        let Some(intersection) = self.intersection(other) else {
            return 0;
        };
        let union = self.area() + other.area() - intersection.area();
        ((intersection.area() * 1_000) / union).min(1_000) as u16
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x.saturating_add((self.width / 2) as i32),
            y: self.y.saturating_add((self.height / 2) as i32),
        }
    }
}

/// A fixed-point homogeneous 2D affine transform. Matrix coefficients are permille;
/// translations are pixels. Integer arithmetic keeps results identical across targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Affine2D {
    pub m11_permille: i32,
    pub m12_permille: i32,
    pub m21_permille: i32,
    pub m22_permille: i32,
    pub tx_px: i32,
    pub ty_px: i32,
}

impl Affine2D {
    pub const IDENTITY: Self = Self {
        m11_permille: 1_000,
        m12_permille: 0,
        m21_permille: 0,
        m22_permille: 1_000,
        tx_px: 0,
        ty_px: 0,
    };

    pub const fn translation(tx_px: i32, ty_px: i32) -> Self {
        Self {
            tx_px,
            ty_px,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x_permille: i32, y_permille: i32) -> Result<Self, GeometryError> {
        if x_permille <= 0 || y_permille <= 0 || x_permille > 16_000 || y_permille > 16_000 {
            return Err(GeometryError::InvalidTransform);
        }
        Ok(Self {
            m11_permille: x_permille,
            m22_permille: y_permille,
            ..Self::IDENTITY
        })
    }

    pub fn apply_point(&self, point: Point) -> Result<Point, GeometryError> {
        let x = i64::from(self.m11_permille) * i64::from(point.x)
            + i64::from(self.m12_permille) * i64::from(point.y);
        let y = i64::from(self.m21_permille) * i64::from(point.x)
            + i64::from(self.m22_permille) * i64::from(point.y);
        let x = x / SCALE + i64::from(self.tx_px);
        let y = y / SCALE + i64::from(self.ty_px);
        if x.unsigned_abs() > MAX_COORDINATE as u64 || y.unsigned_abs() > MAX_COORDINATE as u64 {
            return Err(GeometryError::Overflow);
        }
        Ok(Point {
            x: i32::try_from(x).map_err(|_| GeometryError::Overflow)?,
            y: i32::try_from(y).map_err(|_| GeometryError::Overflow)?,
        })
    }

    pub fn apply_rect(&self, rect: Rect) -> Result<Rect, GeometryError> {
        rect.validate()?;
        let corners = [
            Point {
                x: rect.x,
                y: rect.y,
            },
            Point {
                x: rect.right() as i32,
                y: rect.y,
            },
            Point {
                x: rect.x,
                y: rect.bottom() as i32,
            },
            Point {
                x: rect.right() as i32,
                y: rect.bottom() as i32,
            },
        ];
        let transformed = corners
            .into_iter()
            .map(|point| self.apply_point(point))
            .collect::<Result<Vec<_>, _>>()?;
        let left = transformed.iter().map(|point| point.x).min().unwrap();
        let top = transformed.iter().map(|point| point.y).min().unwrap();
        let right = transformed.iter().map(|point| point.x).max().unwrap();
        let bottom = transformed.iter().map(|point| point.y).max().unwrap();
        Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_and_screen_transforms_are_integer_and_reproducible() {
        let roi = Rect::new(100, 200, 40, 20).unwrap();
        assert_eq!(
            Affine2D::translation(20, -30).apply_rect(roi).unwrap(),
            Rect::new(120, 170, 40, 20).unwrap()
        );
        assert_eq!(
            Affine2D::scale(1_500, 2_000)
                .unwrap()
                .apply_point(Point { x: 10, y: 10 })
                .unwrap(),
            Point { x: 15, y: 20 }
        );
    }

    #[test]
    fn overlap_is_bounded() {
        let a = Rect::new(0, 0, 100, 100).unwrap();
        let b = Rect::new(50, 0, 100, 100).unwrap();
        assert_eq!(a.iou_permille(&b), 333);
        assert_eq!(a.iou_permille(&a), 1_000);
    }
}
