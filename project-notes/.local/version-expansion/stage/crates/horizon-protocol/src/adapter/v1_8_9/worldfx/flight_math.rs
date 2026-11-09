//! Legacy47 fireball's fixed direction and move-before-inertia math, derived from vanilla ws.
pub fn acceleration_from_wire(vector: [f64; 3]) -> Option<[f64; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    // MathHelper.sqrt_double in vanilla1.8 returns float, then the constructor widens it to double.
    let length = (vector.iter().map(|value| value * value).sum::<f64>().sqrt() as f32) as f64;
    if length == 0. || !length.is_finite() {
        return None;
    }
    Some(vector.map(|value| value / length * 0.1))
}
pub fn next_velocity(
    velocity: [f64; 3],
    acceleration: [f64; 3],
    water: bool,
    dangerous: bool,
) -> [f64; 3] {
    // The old water branch overrides WitherSkull's dangerous-air inertia.
    let inertia = f64::from(if water {
        0.8_f32
    } else if dangerous {
        0.73_f32
    } else {
        0.95_f32
    });
    std::array::from_fn(|axis| (velocity[axis] + acceleration[axis]) * inertia)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn starts_at_rest_then_accelerates_after_the_move() {
        let acceleration = acceleration_from_wire([1., 0., 0.]).unwrap();
        let mut position = [0.; 3];
        let mut velocity = [0.; 3];
        position = std::array::from_fn(|axis| position[axis] + velocity[axis]);
        velocity = next_velocity(velocity, acceleration, false, false);
        assert_eq!(position, [0.; 3]);
        assert_eq!(velocity, [0.1 * f64::from(0.95_f32), 0., 0.]);
        position = std::array::from_fn(|axis| position[axis] + velocity[axis]);
        assert_eq!(position, velocity);
    }
    #[test]
    fn a_sideways_motion_correction_does_not_rotate_the_old_fixed_acceleration() {
        let next = next_velocity([0., 1., 0.], [0.1, 0., 0.], false, false);
        assert_eq!(next, [0.1 * f64::from(0.95_f32), f64::from(0.95_f32), 0.]);
        assert_eq!(
            next_velocity([0.; 3], [0.1, 0., 0.], true, true)[0],
            0.1 * f64::from(0.8_f32)
        );
    }
    #[test]
    fn undefined_zero_and_nonfinite_directions_are_explicitly_rejected() {
        assert_eq!(acceleration_from_wire([0.; 3]), None);
        assert_eq!(acceleration_from_wire([f64::NAN, 0., 0.]), None);
    }
}
