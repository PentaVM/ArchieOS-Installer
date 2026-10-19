//! XaviCottier field.js integration with exact rest and separate dirty/active flags.
use std::ops::{Add, Mul, Sub};
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DesignVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl DesignVec3 {
    pub fn dot(self, b: Self) -> f32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    pub fn unit(self) -> Self {
        let length = self.length();
        if length > 0. {
            self * (1. / length)
        } else {
            Self::default()
        }
    }
    pub fn cross(self, b: Self) -> Self {
        Self {
            x: self.y * b.z - self.z * b.y,
            y: self.z * b.x - self.x * b.z,
            z: self.x * b.y - self.y * b.x,
        }
    }
}
impl Add for DesignVec3 {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self {
            x: self.x + b.x,
            y: self.y + b.y,
            z: self.z + b.z,
        }
    }
}
impl Sub for DesignVec3 {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self {
            x: self.x - b.x,
            y: self.y - b.y,
            z: self.z - b.z,
        }
    }
}
impl Mul<f32> for DesignVec3 {
    type Output = Self;
    fn mul(self, b: f32) -> Self {
        Self {
            x: self.x * b,
            y: self.y * b,
            z: self.z * b,
        }
    }
}
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DesignBody {
    pub position: DesignVec3,
    pub velocity: DesignVec3,
    pub anchor: DesignVec3,
    pub fixed: u32,
}
impl DesignBody {
    pub fn valid(self) -> bool {
        [self.position, self.velocity, self.anchor]
            .into_iter()
            .all(|v| v.finite() && v.x.abs() <= 1e6 && v.y.abs() <= 1e6 && v.z.abs() <= 1e6)
    }
}
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignMotion {
    pub radius: f32,
    pub spring: f32,
    pub damping: f32,
    pub reach: f32,
    pub reaction: f32,
}
impl Default for DesignMotion {
    fn default() -> Self {
        Self {
            radius: 200.,
            spring: 23.,
            damping: 0.83,
            reach: 50.,
            reaction: 95.,
        }
    }
}
impl DesignMotion {
    pub fn valid(self) -> bool {
        self.radius.is_finite()
            && (0. ..=1e6).contains(&self.radius)
            && self.spring.is_finite()
            && (0. ..=1e6).contains(&self.spring)
            && self.damping.is_finite()
            && (0. ..=1.).contains(&self.damping)
            && self.reach.is_finite()
            && (0. ..=1e6).contains(&self.reach)
            && self.reaction.is_finite()
            && self.reaction.abs() <= 1e6
    }
}
pub const POSITIONS_CHANGED: u32 = 1;
pub const SIMULATION_ACTIVE: u32 = 2;
pub const MAX_BODIES: usize = 36_000;
const REST_DISTANCE: f32 = 0.0001;
/// Rejects invalid input atomically, before changing any body. An active zero-length
/// time step stays awake; callers can distinguish movement from remaining energy.
pub fn integrate(
    bodies: &mut [DesignBody],
    pointer: Option<DesignVec3>,
    delta: f32,
    m: DesignMotion,
) -> u32 {
    if bodies.len() > MAX_BODIES
        || !delta.is_finite()
        || !m.valid()
        || pointer.is_some_and(|p| !p.finite())
        || bodies.iter().any(|b| !b.valid())
    {
        return 0;
    }
    let dt = delta.clamp(0., 0.05);
    if dt == 0. {
        return SIMULATION_ACTIVE;
    }
    let damping = m.damping.powf(dt * 60.);
    let radius_squared = m.radius * m.radius;
    let focus_active = pointer.is_some() && m.reaction != 0. && m.reach > 0.;
    let rest_squared = REST_DISTANCE * REST_DISTANCE;
    let free_travel_rate = if m.damping > 0. && m.damping < 1. {
        -60. * m.damping.ln()
    } else {
        0.
    };
    let mut result = 0;
    for b in bodies {
        let previous = b.position;
        if b.fixed != 0 {
            b.position = b.anchor;
            b.velocity = DesignVec3::default();
            if b.position != previous {
                result |= POSITIONS_CHANGED;
            }
            continue;
        }
        if !focus_active && m.spring > 0. && damping > 0. && damping <= 1. {
            let mut target = b.anchor;
            let target_squared = target.dot(target);
            let constrained = target_squared > radius_squared;
            if constrained {
                target = target * (m.radius / target_squared.sqrt());
            }
            let displacement = target - b.position;
            let displacement_squared = displacement.dot(displacement);
            let mut velocity = b.velocity;
            if constrained && m.radius > 0. && displacement_squared <= rest_squared {
                let outward = (velocity.dot(target) / radius_squared).max(0.);
                velocity = velocity - target * outward;
            }
            let speed_squared = velocity.dot(velocity);
            if (displacement_squared == 0. && speed_squared == 0.)
                || (damping < 1. && displacement_squared + speed_squared / m.spring <= rest_squared)
            {
                b.position = target;
                b.velocity = DesignVec3::default();
                if b.position != previous {
                    result |= POSITIONS_CHANGED;
                }
                continue;
            }
        }
        if !focus_active
            && m.spring == 0.
            && free_travel_rate > 0.
            && b.velocity.dot(b.velocity) <= rest_squared * free_travel_rate * free_travel_rate
        {
            b.velocity = DesignVec3::default();
        }
        let mut acceleration = (b.anchor - b.position) * m.spring;
        if focus_active {
            let mut away = b.position - pointer.unwrap_or_default();
            let mut distance = away.length();
            if distance < 0.001 {
                away = DesignVec3 {
                    x: if b.anchor.x != 0. { b.anchor.x } else { 1. },
                    y: b.anchor.y,
                    z: b.anchor.z,
                }
                .unit();
                distance = 1.;
            }
            let falloff = (1. - distance / m.reach).max(0.);
            acceleration = acceleration + away * (m.reaction * falloff * falloff / distance);
        }
        b.velocity = (b.velocity + acceleration * dt) * damping;
        b.position = b.position + b.velocity * dt;
        let distance = b.position.length();
        if distance > m.radius {
            b.position = b.position * (m.radius / distance);
            b.velocity = b.velocity * 0.3;
        }
        if b.position != previous {
            result |= POSITIONS_CHANGED;
        }
        if damping != 0.
            && (focus_active
                || b.velocity != DesignVec3::default()
                || (m.spring != 0. && b.position != b.anchor))
        {
            result |= SIMULATION_ACTIVE;
        }
    }
    result
}
#[derive(Debug, Clone, Copy, PartialEq)]
struct InputKey {
    ptr: usize,
    count: usize,
    pointer: Option<DesignVec3>,
    motion: DesignMotion,
    revision: u64,
}
#[derive(Default)]
pub struct Simulation {
    sleeping: bool,
    key: Option<InputKey>,
}
impl Simulation {
    /// revision must change after modifying anchors, velocities or positions in place.
    /// A sleeping field checks only this key, never enumerating its bodies.
    pub fn step(
        &mut self,
        bodies: &mut [DesignBody],
        pointer: Option<DesignVec3>,
        dt: f32,
        motion: DesignMotion,
        revision: u64,
    ) -> u32 {
        let key = InputKey {
            ptr: bodies.as_ptr() as usize,
            count: bodies.len(),
            pointer,
            motion,
            revision,
        };
        if self.key != Some(key) {
            self.wake();
            self.key = Some(key);
        }
        if self.sleeping || dt <= 0. {
            return 0;
        }
        let result = integrate(bodies, pointer, dt, motion);
        self.sleeping = result & SIMULATION_ACTIVE == 0;
        result
    }
    pub fn wake(&mut self) {
        self.sleeping = false;
    }
    pub fn sleeping(&self) -> bool {
        self.sleeping
    }
}
