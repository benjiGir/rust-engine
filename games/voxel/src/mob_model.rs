//! Géométrie et articulations des créatures, communes au jeu et aux planches de contrôle.
use engine_render::CubeInstance;
use engine_voxel::BlockRegistry;
use glam::Vec3;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobPart {
    pub offset: (f32, f32, f32),
    pub size: (f32, f32, f32),
    pub color: (f32, f32, f32),
    #[serde(default)]
    pub bone: Bone,
    /// Texture optionnelle, qualifiée par l'espace de noms de la créature.
    #[serde(default)]
    pub texture: Option<String>,
    #[serde(skip)]
    pub texture_layer: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Bone {
    #[default]
    Fixed,
    Leg,
    Arm,
    Head,
    /// Queue et nageoire caudale : pivot commun, oscillation latérale pendant la nage.
    Tail,
}

pub fn bind_textures(parts: &mut [MobPart], namespace: &str, blocks: &mut BlockRegistry) {
    for part in parts {
        part.texture_layer = part.texture.as_ref().map_or(0, |name| {
            let key = if name.contains(':') {
                name.clone()
            } else {
                format!("{namespace}:{name}")
            };
            blocks.register_texture(&key)
        });
    }
}

/// Modèle local, +X vers l'avant. Les textures restent attachées aux boîtes animées.
pub fn instances(
    parts: &[MobPart],
    phase: f32,
    stride: f32,
    look: f32,
) -> impl Iterator<Item = CubeInstance> + '_ {
    let volume = |p: &MobPart| p.size.0 * p.size.1 * p.size.2;
    let neck = parts
        .iter()
        .filter(|p| p.bone == Bone::Head)
        .max_by(|a, b| volume(a).total_cmp(&volume(b)))
        .map(|p| {
            (
                p.offset.0
                    - if p.offset.0 > 0.3 {
                        p.size.0 * 0.4
                    } else {
                        0.0
                    },
                p.offset.2,
            )
        });
    let tail = parts
        .iter()
        .filter(|p| p.bone == Bone::Tail)
        .max_by(|a, b| volume(a).total_cmp(&volume(b)))
        .map(|p| (p.offset.0 + p.size.0 * 0.4, p.offset.2));
    parts.iter().map(move |part| {
        let (mut x, y, mut z) = part.offset;
        let mut yaw = 0.0;
        let mut swing = [0.0; 2];
        match part.bone {
            Bone::Fixed => {}
            Bone::Head => {
                if let Some((nx, nz)) = neck {
                    let (rx, rz) = (x - nx, z - nz);
                    x = nx + rx * look.cos() - rz * look.sin();
                    z = nz + rx * look.sin() + rz * look.cos();
                    yaw = look;
                }
            }
            Bone::Tail => {
                if let Some((nx, nz)) = tail {
                    let angle = (0.18 + stride * 0.45) * (phase * 1.7).sin();
                    let (rx, rz) = (x - nx, z - nz);
                    x = nx + rx * angle.cos() - rz * angle.sin();
                    z = nz + rx * angle.sin() + rz * angle.cos();
                    yaw = angle;
                }
            }
            Bone::Leg | Bone::Arm => {
                let left = part.offset.2 > 0.0;
                let opposite = if part.bone == Bone::Leg {
                    (part.offset.0 > 0.01) ^ left
                } else {
                    !left
                };
                let offset = if opposite { std::f32::consts::PI } else { 0.0 };
                let hip = parts
                    .iter()
                    .filter(|q| {
                        q.bone == part.bone
                            && (q.offset.0 - part.offset.0).abs() < 0.02
                            && (q.offset.2 - part.offset.2).abs() < 0.02
                    })
                    .map(|q| q.offset.1 + q.size.1 * 0.5)
                    .fold(f32::MIN, f32::max);
                swing = [0.65 * stride * (phase + offset).sin(), hip - y];
            }
        }
        let tint = if part.texture_layer == 0 {
            Vec3::new(part.color.0, part.color.1, part.color.2)
        } else {
            Vec3::ONE
        };
        let mut cube = CubeInstance::new(
            Vec3::new(x, y, z),
            Vec3::new(part.size.0, part.size.1, part.size.2),
            tint,
        )
        .with_yaw(yaw);
        cube.textures = [part.texture_layer; 3];
        cube.swing = swing;
        cube
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_and_caudal_fin_keep_a_common_pivot_during_the_swim() {
        let parts: Vec<MobPart> = ron::from_str(
            r#"[
          (offset: (-0.25,0.22,0.0),size: (0.22,0.15,0.1),color: (0.3,0.4,0.2),bone: Tail),
          (offset: (-0.40,0.22,0.0),size: (0.13,0.28,0.045),color: (0.4,0.3,0.1),bone: Tail)
        ]"#,
        )
        .unwrap();
        let a: Vec<_> = instances(&parts, 0.0, 0.8, 0.0).collect();
        let b: Vec<_> = instances(&parts, 1.0, 0.8, 0.0).collect();
        assert_eq!(b[0].yaw, b[1].yaw);
        assert!(b[0].yaw.abs() > 0.1);
        assert!(
            (Vec3::from_array(a[0].center).distance(Vec3::from_array(a[1].center))
                - Vec3::from_array(b[0].center).distance(Vec3::from_array(b[1].center)))
            .abs()
                < 1e-5
        );
    }
    #[test]
    fn textured_hoof_and_leg_share_their_hip_and_head_details_share_the_neck() {
        let mut parts: Vec<MobPart> = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME).from_str(r#"[
            (offset: (0.3, 0.3, 0.2), size: (0.2, 0.6, 0.2), color: (0.4, 0.2, 0.1), bone: Leg, texture: "coat"),
            (offset: (0.3, 0.05, 0.2), size: (0.2, 0.1, 0.2), color: (0.1, 0.1, 0.1), bone: Leg),
            (offset: (0.7, 1.0, 0.0), size: (0.5, 0.4, 0.5), color: (0.4, 0.2, 0.1), bone: Head),
            (offset: (1.0, 1.0, 0.15), size: (0.05, 0.04, 0.04), color: (0.1, 0.1, 0.1), bone: Head)
        ]"#).unwrap();
        let mut blocks = BlockRegistry::empty();
        bind_textures(&mut parts, "test", &mut blocks);
        let posed: Vec<_> = instances(&parts, 1.0, 1.0, 0.6).collect();
        assert_eq!(posed[0].textures, [1; 3]);
        assert_eq!(posed[0].color, [1.0; 3]);
        assert_eq!(posed[1].textures, [0; 3]);
        assert_eq!(posed[0].swing[0], posed[1].swing[0]);
        assert!(
            (posed[0].center[1] + posed[0].swing[1] - posed[1].center[1] - posed[1].swing[1]).abs()
                < 1e-5
        );
        assert_eq!(posed[2].yaw, posed[3].yaw);
        let distance = |a: [f32; 3], b: [f32; 3]| Vec3::from_array(a).distance(Vec3::from_array(b));
        assert!(
            (distance(posed[2].center, posed[3].center) - Vec3::new(0.3, 0.0, 0.15).length()).abs()
                < 1e-5
        );
    }
}
