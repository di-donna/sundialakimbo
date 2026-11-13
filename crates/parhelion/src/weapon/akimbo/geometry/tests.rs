use super::*;

#[test]
fn packed_two_bone_geometry_preserves_weights_and_checks_every_influence() {
    let mut positions = Vec::new();
    for (index, row) in stock_positions().chunks_exact(8).enumerate() {
        positions.extend_from_slice(&row[..6]);
        positions.extend_from_slice(&0u16.to_le_bytes());
        let weight = [0, 127, 255, 1][index];
        positions.extend([0, 2, weight, 255 - weight]);
    }
    let attributes = vec![17; 80];
    let indices = [0, 1, 2, 2, 3, 0];
    let draws = [triangle_list_draw()];
    let build = |bytes: &[u8]| {
        let parts = [Part {
            position_stride: 12,
            ..part(bytes, &attributes, &indices, &draws)
        }];
        prepare(source(&parts), settings())
    };
    let result = build(&positions).unwrap();
    let output = &result.parts[0];
    assert_eq!(output.positions.len(), 96);
    assert_eq!(output.indices, [0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]);
    for (index, row) in output.positions.chunks_exact(12).enumerate() {
        assert_eq!(
            &row[8..],
            &positions[(index % 4) * 12 + 8..(index % 4) * 12 + 12]
        );
        assert_eq!(
            u16::from_le_bytes([row[6], row[7]]),
            if index < 4 { 0 } else { LEFT_COPY_FLAGS }
        );
        for axis in 0..3 {
            let at = (index % 4) * 12 + axis * 2;
            let source_value = i16::from_le_bytes(positions[at..at + 2].try_into().unwrap()) as f64
                / POSITION_UNIT
                * 0.5;
            let expected = source_value
                + if axis == 1 && index >= 4 {
                    f64::from(settings().separation)
                } else {
                    0.0
                };
            let actual = i16::from_le_bytes(row[axis * 2..axis * 2 + 2].try_into().unwrap()) as f64
                / POSITION_UNIT
                * f64::from(result.position_scale);
            assert!((actual - expected).abs() <= f64::from(settings().max_position_error));
        }
    }
    for (offset, value, error) in [
        (10, 1, GeometryError::InvalidWeights),
        (9, 64, GeometryError::InvalidBoneLane),
        (8, 5, GeometryError::OccupiedMarker),
        (6, 1, GeometryError::InvalidBoneLane),
    ] {
        let mut changed = positions.clone();
        changed[offset] = value;
        assert_eq!(build(&changed), Err(error));
    }
}

fn position_rows(rows: &[[i16; 4]]) -> Vec<u8> {
    rows.iter()
        .flat_map(|row| row.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn stock_positions() -> Vec<u8> {
    position_rows(&[
        [-24000, -32000, 1000, 0],
        [30000, 32000, -17000, 2],
        [10000, -100, 11000, 3],
        [-32768, 32767, 32767, 0],
    ])
}

fn settings() -> Settings {
    Settings {
        separation: 0.36,
        marker_bone: 5,
        max_position_error: 0.00004,
    }
}

fn source<'a>(parts: &'a [Part<'a>]) -> Source<'a> {
    Source {
        position_scale: 0.5,
        position_offset: [0.1, -0.2, 0.3],
        bone_palette: 8,
        parts,
    }
}

fn triangle_list_draw() -> Draw {
    Draw {
        first_index: 0,
        index_count: 6,
        primitive_count: 2,
        topology: Topology::TriangleList,
        duplicate: true,
    }
}

fn part<'a>(
    positions: &'a [u8],
    attributes: &'a [u8],
    indices: &'a [u16],
    draws: &'a [Draw],
) -> Part<'a> {
    Part {
        positions,
        position_stride: 8,
        attributes,
        attribute_stride: 20,
        indices,
        draws,
        duplicate: true,
    }
}

fn decoded(bytes: &[u8], scale: f32, offset: [f32; 3]) -> Vec<([f64; 3], u16)> {
    bytes
        .chunks_exact(8)
        .map(|row| {
            let point = std::array::from_fn(|axis| {
                f64::from(i16::from_le_bytes([row[axis * 2], row[axis * 2 + 1]])) / POSITION_UNIT
                    * f64::from(scale)
                    + f64::from(offset[axis])
            });
            (point, u16::from_le_bytes([row[6], row[7]]))
        })
        .collect()
}

#[test]
fn both_hands_preserve_geometry_attributes_and_triangle_winding() {
    let positions = stock_positions();
    let attributes = (0..80).collect::<Vec<u8>>();
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let native = source(&parts);
    let output = prepare(native, settings()).unwrap();
    let before = decoded(&positions, native.position_scale, native.position_offset);
    let after = decoded(
        &output.parts[0].positions,
        output.position_scale,
        output.position_offset,
    );
    for hand in 0..2 {
        for index in 0..before.len() {
            let (point, bone) = after[hand * before.len() + index];
            assert_eq!(
                bone,
                before[index].1 | if hand == 0 { 0 } else { LEFT_COPY_FLAGS }
            );
            for (axis, coordinate) in point.iter().enumerate() {
                let expected = before[index].0[axis]
                    + if axis == 1 && hand == 1 {
                        f64::from(settings().separation)
                    } else {
                        0.0
                    };
                assert!((*coordinate - expected).abs() <= f64::from(settings().max_position_error));
            }
            let length = (0..3)
                .map(|axis| (point[axis] - f64::from(native.position_offset[axis])).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(length <= f64::from(output.sphere_radius));
        }
    }
    assert_eq!(output.parts[0].attributes, attributes.repeat(2));
    assert_eq!(
        output.parts[0].indices,
        [0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7]
    );
    assert_eq!(
        output.parts[0].draws,
        [Draw {
            index_count: 12,
            primitive_count: 4,
            ..draws[0]
        }]
    );
    assert_eq!(prepare(native, settings()).unwrap(), output);
}

#[test]
fn adjustable_gap_matches_centered_native_pose_without_a_second_mesh_shift() {
    let positions = stock_positions();
    let attributes = vec![19; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let native = source(&parts);
    let original = decoded(&positions, native.position_scale, native.position_offset);
    for gap_mm in [50, 240, 360, 800] {
        let gap = crate::weapon::akimbo::Intent {
            gap_mm,
            ..Default::default()
        }
        .gap_meters();
        let output = prepare(
            native,
            Settings {
                separation: gap,
                ..settings()
            },
        )
        .unwrap();
        let vertices = decoded(
            &output.parts[0].positions,
            output.position_scale,
            output.position_offset,
        );
        let tolerance = 2.0 * f64::from(settings().max_position_error);
        for (index, (point, _)) in original.iter().enumerate() {
            // Native schema 2 applies this common -g/2 placement. The generated buffers
            // must still contain 0/+g, otherwise native placement would center twice.
            let right = vertices[index].0[1] - point[1] - f64::from(gap) / 2.0;
            let left = vertices[index + original.len()].0[1] - point[1] - f64::from(gap) / 2.0;
            assert!(((right + left) / 2.0).abs() <= tolerance);
            assert!((left - right - f64::from(gap)).abs() <= tolerance);
            assert!(right < 0.0 && left > 0.0);
            for axis in [0, 2] {
                assert!((vertices[index].0[axis] - point[axis]).abs() <= tolerance);
                assert!(
                    (vertices[index + original.len()].0[axis] - point[axis]).abs() <= tolerance
                );
            }
        }
    }
}

fn strip_triangles(run: &[u16]) -> Vec<[u16; 3]> {
    let mut triangles = Vec::new();
    for strip in run.split(|&index| index == RESTART) {
        for (index, tri) in strip.windows(3).enumerate() {
            let [a, b, c] = [tri[0], tri[1], tri[2]];
            if a != b && a != c && b != c {
                triangles.push(if index % 2 == 0 { [a, b, c] } else { [b, a, c] });
            }
        }
    }
    triangles
}

#[test]
fn odd_strip_restart_resets_winding_without_bridge_triangles() {
    let positions = stock_positions();
    let attributes = vec![42; 80];
    let indices = [0, 1, 2, RESTART, 3, 2, 1, 0];
    let draws = [Draw {
        first_index: 0,
        index_count: 8,
        primitive_count: 3,
        topology: Topology::TriangleStrip,
        duplicate: true,
    }];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let output = prepare(source(&parts), settings()).unwrap();
    let native = strip_triangles(&indices);
    let mut expected = native.clone();
    expected.extend(native.iter().map(|tri| tri.map(|vertex| vertex + 4)));
    assert_eq!(strip_triangles(&output.parts[0].indices), expected);
    assert_eq!(output.parts[0].draws[0].primitive_count, 6);
    assert_eq!(output.parts[0].indices[8], RESTART);
}

#[test]
fn repeated_material_draw_ranges_share_indices_and_preserve_order() {
    let positions = stock_positions();
    let attributes = vec![12; 80];
    let indices = [0, 1, 2, RESTART, 3, 2, 1];
    let draw_a = Draw {
        first_index: 0,
        index_count: 3,
        primitive_count: 1,
        topology: Topology::TriangleStrip,
        duplicate: true,
    };
    let draw_b = Draw {
        first_index: 4,
        ..draw_a
    };
    // Material/pass order can revisit an earlier range.
    let draws = [draw_b, draw_a, draw_b];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let output = prepare(source(&parts), settings()).unwrap();
    assert_eq!(output.parts[0].indices.len(), 15);
    assert_eq!(output.parts[0].indices[7], RESTART);
    assert_eq!(output.parts[0].draws[0], output.parts[0].draws[2]);
    assert_eq!(output.parts[0].draws[0].first_index, 8);
    assert_eq!(output.parts[0].draws[1].first_index, 0);
}

#[test]
fn retained_reticle_is_requantized_in_the_shared_domain_and_not_duplicated() {
    let positions = stock_positions();
    let attributes = vec![19; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let native_part = part(&positions, &attributes, &indices, &draws);
    let parts = [
        native_part,
        Part {
            duplicate: false,
            ..native_part
        },
    ];
    let output = prepare(source(&parts), settings()).unwrap();
    let retained = &output.parts[1];
    assert_eq!(retained.positions.len(), positions.len());
    assert_eq!(retained.attributes, attributes);
    assert_eq!(retained.indices, indices);
    assert_eq!(retained.draws, draws);
    assert_eq!(
        retained.positions,
        output.parts[0].positions[..positions.len()]
    );
}

#[test]
fn high_native_bones_are_rejected_instead_of_masked_into_copy_flags() {
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    for lane in [8u16, 63, 64, 128, 192, 0x7fff, 0xffff] {
        let mut positions = stock_positions();
        positions[6..8].copy_from_slice(&lane.to_le_bytes());
        let parts = [part(&positions, &attributes, &indices, &draws)];
        assert_eq!(
            prepare(source(&parts), settings()),
            Err(GeometryError::InvalidBoneLane)
        );
    }
}

#[test]
fn marker_is_unweighted_across_retained_parts_too() {
    let positions = stock_positions();
    let mut retained_positions = positions.clone();
    retained_positions[6..8].copy_from_slice(&5u16.to_le_bytes());
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [
        part(&positions, &attributes, &indices, &draws),
        Part {
            duplicate: false,
            ..part(&retained_positions, &attributes, &indices, &draws)
        },
    ];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::OccupiedMarker)
    );
}

#[test]
fn buffer_layouts_and_vertex_index_capacity_are_checked() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let native = part(&positions, &attributes, &indices, &draws);
    for stride in [0, 4, 12, 16] {
        let parts = [Part {
            position_stride: stride,
            ..native
        }];
        assert_eq!(
            prepare(source(&parts), settings()),
            Err(GeometryError::UnsupportedPositionLayout)
        );
    }
    let parts = [Part {
        positions: &positions[..31],
        ..native
    }];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::UnsupportedPositionLayout)
    );
    let parts = [Part {
        attribute_stride: 24,
        ..native
    }];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::InvalidAttributeLayout)
    );
    let too_many = vec![0u8; 32768 * 8];
    let parts = [Part {
        positions: &too_many,
        ..native
    }];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::TooManyVertices)
    );
    let bad_indices = [0, 1, 4];
    let parts = [Part {
        indices: &bad_indices,
        ..native
    }];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::InvalidIndex)
    );
}

#[test]
fn nonfinite_or_unbounded_transforms_are_rejected() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 2.0] {
        assert_eq!(
            prepare(
                source(&parts),
                Settings {
                    separation: value,
                    ..settings()
                }
            ),
            Err(GeometryError::InvalidSeparation)
        );
    }
    for value in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
        assert_eq!(
            prepare(
                Source {
                    position_scale: value,
                    ..source(&parts)
                },
                settings()
            ),
            Err(GeometryError::InvalidTransform)
        );
        assert_eq!(
            prepare(
                source(&parts),
                Settings {
                    max_position_error: value,
                    ..settings()
                }
            ),
            Err(GeometryError::InvalidErrorLimit)
        );
    }
    assert_eq!(
        prepare(
            Source {
                position_offset: [0.0, f32::NAN, 0.0],
                ..source(&parts)
            },
            settings()
        ),
        Err(GeometryError::InvalidTransform)
    );
    assert_eq!(
        prepare(
            source(&parts),
            Settings {
                max_position_error: 1e-12,
                ..settings()
            }
        ),
        Err(GeometryError::QuantizationError)
    );
}

#[test]
fn no_unused_marker_or_no_selected_part_refuses_output() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    for marker in [0, 8, 64] {
        assert_eq!(
            prepare(
                source(&parts),
                Settings {
                    marker_bone: marker,
                    ..settings()
                }
            ),
            Err(GeometryError::InvalidMarker)
        );
    }
    for palette in [0, 65] {
        assert_eq!(
            prepare(
                Source {
                    bone_palette: palette,
                    ..source(&parts)
                },
                settings()
            ),
            Err(GeometryError::InvalidPalette)
        );
    }
    let parts = [Part {
        duplicate: false,
        ..parts[0]
    }];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::NoDuplicatedPart)
    );
    assert_eq!(
        prepare(source(&[]), settings()),
        Err(GeometryError::EmptyModel)
    );
}

#[test]
fn invalid_primitive_metadata_and_out_of_bounds_ranges_are_rejected() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    for draw in [
        Draw {
            primitive_count: 3,
            ..triangle_list_draw()
        },
        Draw {
            primitive_count: 0,
            ..triangle_list_draw()
        },
    ] {
        let draws = [draw];
        let parts = [part(&positions, &attributes, &indices, &draws)];
        assert_eq!(
            prepare(source(&parts), settings()),
            Err(GeometryError::InvalidPrimitiveCount)
        );
    }
    for (draw, error) in [
        (
            Draw {
                first_index: u32::MAX,
                ..triangle_list_draw()
            },
            GeometryError::InvalidDrawRange,
        ),
        (
            Draw {
                first_index: 3,
                ..triangle_list_draw()
            },
            GeometryError::InvalidDrawRange,
        ),
        (
            Draw {
                index_count: 0,
                ..triangle_list_draw()
            },
            GeometryError::EmptyDraw,
        ),
        (
            Draw {
                index_count: 5,
                ..triangle_list_draw()
            },
            GeometryError::InvalidTriangleList,
        ),
    ] {
        let draws = [draw];
        let parts = [part(&positions, &attributes, &indices, &draws)];
        assert_eq!(prepare(source(&parts), settings()), Err(error));
    }
}

#[test]
fn foreign_indices_and_overlapping_ranges_are_rejected() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [Draw {
        first_index: 3,
        index_count: 3,
        primitive_count: 1,
        topology: Topology::TriangleList,
        duplicate: true,
    }];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::UnownedIndices)
    );
    let draws = [triangle_list_draw(), draws[0]];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::OverlappingDrawRanges)
    );
}

#[test]
fn native_degenerate_count_convention_is_preserved() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 3];
    for primitive_count in 1..=3 {
        let draws = [Draw {
            first_index: 0,
            index_count: 5,
            primitive_count,
            topology: Topology::TriangleStrip,
            duplicate: true,
        }];
        let parts = [part(&positions, &attributes, &indices, &draws)];
        let output = prepare(source(&parts), settings()).unwrap();
        assert_eq!(
            output.parts[0].draws[0].primitive_count,
            primitive_count * 2
        );
        assert_eq!(strip_triangles(&output.parts[0].indices).len(), 2);
    }
}

#[test]
fn record_mask_capacity_covers_highest_id_not_number_of_records() {
    assert_eq!(record_mask_capacity(&[]), 0);
    assert_eq!(record_mask_capacity(&[0, 0]), 32);
    assert_eq!(record_mask_capacity(&[31]), 32);
    assert_eq!(record_mask_capacity(&[32, 1]), 64);
    assert_eq!(record_mask_capacity(&[65, 3]), 96);
    assert_eq!(record_mask_capacity(&[u16::MAX]), 65536);
}

#[test]
fn quantization_is_independent_of_the_absolute_model_offset() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [triangle_list_draw()];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let normal = prepare(source(&parts), settings()).unwrap();
    let moved = prepare(
        Source {
            position_offset: [1e20, -1e20, 1e20],
            ..source(&parts)
        },
        settings(),
    )
    .unwrap();
    assert_eq!(normal.parts, moved.parts);
    assert_eq!(normal.position_scale, moved.position_scale);
    assert_eq!(normal.sphere_radius, moved.sphere_radius);
    assert_eq!(normal.max_position_error, moved.max_position_error);
}

#[test]
fn a_shared_range_cannot_change_primitive_topology_between_draws() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2];
    let draw = Draw {
        first_index: 0,
        index_count: 3,
        primitive_count: 1,
        topology: Topology::TriangleList,
        duplicate: true,
    };
    let draws = [
        draw,
        Draw {
            topology: Topology::TriangleStrip,
            duplicate: true,
            ..draw
        },
    ];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::ConflictingTopology)
    );
}

#[test]
fn paired_geometry_and_retained_reticle_share_streams_without_duplicate_reticle_draws() {
    let positions = stock_positions();
    let attributes = vec![37; 80];
    let indices = [0, 1, 2, RESTART, 0, 1, 2, 3];
    let reticle = Draw {
        first_index: 0,
        index_count: 3,
        primitive_count: 1,
        topology: Topology::TriangleList,
        duplicate: false,
    };
    let paired = Draw {
        first_index: 4,
        index_count: 4,
        primitive_count: 2,
        topology: Topology::TriangleStrip,
        duplicate: true,
    };
    // Native material/stage draws may share a range; preserve their order and reuse the
    // transformed range once. Retained and paired records need not be sorted by index.
    let draws = [paired, reticle, paired];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let prepared = prepare(source(&parts), settings()).unwrap();
    let result = &prepared.parts[0];
    assert_eq!(result.positions.len(), positions.len() * 2);
    assert_eq!(result.attributes, attributes.repeat(2));
    assert_eq!(
        result.indices,
        [0, 1, 2, RESTART, 0, 1, 2, 3, RESTART, 4, 5, 6, 7]
    );
    assert_eq!(result.draws[1], reticle);
    assert_eq!(result.draws[0].first_index, 4);
    assert_eq!(result.draws[0].index_count, 9);
    assert_eq!(result.draws[0].primitive_count, 4);
    assert_eq!(result.draws[0], result.draws[2]);
}

#[test]
fn retained_draws_after_a_pair_relocate_without_changing_their_geometry() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let draws = [
        Draw {
            index_count: 3,
            primitive_count: 1,
            ..triangle_list_draw()
        },
        Draw {
            first_index: 3,
            index_count: 3,
            primitive_count: 1,
            duplicate: false,
            ..triangle_list_draw()
        },
    ];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    let prepared = prepare(source(&parts), settings()).unwrap();
    assert_eq!(prepared.parts[0].indices, [0, 1, 2, 4, 5, 6, 2, 1, 3]);
    assert_eq!(
        prepared.parts[0].draws[1],
        Draw {
            first_index: 6,
            ..draws[1]
        }
    );
}

#[test]
fn rejects_ambiguous_range_duplication_and_models_with_no_paired_draw() {
    let positions = stock_positions();
    let attributes = vec![0; 80];
    let indices = [0, 1, 2, 2, 1, 3];
    let retained = Draw {
        duplicate: false,
        ..triangle_list_draw()
    };
    let draws = [triangle_list_draw(), retained];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::ConflictingDuplication)
    );
    let draws = [retained];
    let parts = [part(&positions, &attributes, &indices, &draws)];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::NoDuplicatedPart)
    );
}

#[test]
fn model_output_budget_is_checked_before_preparing_parts() {
    let positions = vec![0; 32767 * 8];
    let attributes = vec![0; 32767 * 128];
    let indices = [0, 1, 2];
    let draws = [Draw {
        first_index: 0,
        index_count: 3,
        primitive_count: 1,
        topology: Topology::TriangleList,
        duplicate: true,
    }];
    let part = Part {
        attribute_stride: 128,
        ..part(&positions, &attributes, &indices, &draws)
    };
    let parts = [part; 8];
    assert_eq!(
        prepare(source(&parts), settings()),
        Err(GeometryError::OutputLimit)
    );
}
