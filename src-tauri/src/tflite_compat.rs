//! Fixed-model importer additions. This framework is used only after checking
//! the bundled Face Landmarker archive's exact size and SHA-256.
use tract_tflite::{
    internal::tract_core::{
        internal::ensure,
        ops::{cast::cast, change_axes::wire_with_rank_broadcast, math},
    },
    prelude::*,
    registry::DeserOp,
    tflite::BuiltinOperator,
};
pub fn framework() -> tract_tflite::Tflite {
    let mut f = tract_tflite::tflite();
    f.0.reg_to_tract(BuiltinOperator::DEQUANTIZE, |op| {
        ensure!(op.inputs.len() == 1);
        ensure!(op.facts()?[0].datum_type == DatumType::F16);
        op.ctx
            .target
            .wire_node(op.prefix, cast(DatumType::F32), op.inputs)
    });
    f.0.reg_to_tract(BuiltinOperator::PRELU, prelu);
    f.0.reg_to_tract(BuiltinOperator::NEG, |op| {
        ensure!(op.inputs.len() == 1);
        op.ctx.target.wire_node(op.prefix, math::neg(), op.inputs)
    });
    f.0.reg_to_tract(BuiltinOperator::SQUARED_DIFFERENCE, |op| {
        ensure!(op.inputs.len() == 2);
        let d = wire_with_rank_broadcast(
            format!("{}.diff", op.prefix),
            op.ctx.target,
            math::sub(),
            op.inputs,
        )?;
        wire_with_rank_broadcast(op.prefix, op.ctx.target, math::mul(), &[d[0], d[0]])
    });
    f
}
fn prelu(op: &mut DeserOp) -> TractResult<TVec<OutletId>> {
    ensure!(op.inputs.len() == 2);
    let zero = op
        .ctx
        .target
        .add_const(format!("{}.zero", op.prefix), tensor0(0f32))?;
    let positive = wire_with_rank_broadcast(
        format!("{}.positive", op.prefix),
        op.ctx.target,
        math::max(),
        &[op.inputs[0], zero],
    )?;
    let negative = wire_with_rank_broadcast(
        format!("{}.negative", op.prefix),
        op.ctx.target,
        math::min(),
        &[op.inputs[0], zero],
    )?;
    let scaled = wire_with_rank_broadcast(
        format!("{}.scaled", op.prefix),
        op.ctx.target,
        math::mul(),
        &[negative[0], op.inputs[1]],
    )?;
    wire_with_rank_broadcast(
        op.prefix,
        op.ctx.target,
        math::add(),
        &[positive[0], scaled[0]],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;
    use tract_tflite::tflite as fb;
    // Tiny external-format graphs test deserialization, not just Tract's own
    // operators. These cases regressed in the published importer.
    fn graph(
        op: BuiltinOperator,
        input_shape: &[i32],
        output_shape: &[i32],
        constants: &[Vec<i32>],
        shapes: &[Vec<i32>],
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let mut buffers = vec![fb::Buffer::create(&mut b, &fb::BufferArgs::default())];
        for values in constants {
            let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            let data = b.create_vector(&bytes);
            buffers.push(fb::Buffer::create(
                &mut b,
                &fb::BufferArgs {
                    data: Some(data),
                    ..Default::default()
                },
            ));
        }
        let mut tensors = vec![];
        for i in 0..constants.len() + 2 {
            let dims = if i == 0 {
                input_shape
            } else if i == constants.len() + 1 {
                output_shape
            } else {
                &shapes[i - 1]
            };
            let shape = if dims.is_empty() {
                None
            } else {
                Some(b.create_vector(dims))
            };
            let name = b.create_string(&format!("tensor-{i}"));
            tensors.push(fb::Tensor::create(
                &mut b,
                &fb::TensorArgs {
                    shape,
                    type_: if i > 0 && i <= constants.len() {
                        fb::TensorType::INT32
                    } else {
                        fb::TensorType::FLOAT32
                    },
                    buffer: if i > 0 && i <= constants.len() {
                        i as u32
                    } else {
                        0
                    },
                    name: Some(name),
                    has_rank: true,
                    ..Default::default()
                },
            ));
        }
        let (option_type, options) = if op == BuiltinOperator::MEAN {
            (
                fb::BuiltinOptions::ReducerOptions,
                Some(
                    fb::ReducerOptions::create(
                        &mut b,
                        &fb::ReducerOptionsArgs { keep_dims: false },
                    )
                    .as_union_value(),
                ),
            )
        } else if op == BuiltinOperator::STRIDED_SLICE {
            (
                fb::BuiltinOptions::StridedSliceOptions,
                Some(
                    fb::StridedSliceOptions::create(
                        &mut b,
                        &fb::StridedSliceOptionsArgs {
                            shrink_axis_mask: 2,
                            ..Default::default()
                        },
                    )
                    .as_union_value(),
                ),
            )
        } else {
            (fb::BuiltinOptions::NONE, None)
        };
        let ins = b.create_vector(&(0..=constants.len() as i32).collect::<Vec<_>>());
        let outs = b.create_vector(&[constants.len() as i32 + 1]);
        let node = fb::Operator::create(
            &mut b,
            &fb::OperatorArgs {
                inputs: Some(ins),
                outputs: Some(outs),
                builtin_options_type: option_type,
                builtin_options: options,
                ..Default::default()
            },
        );
        let code = fb::OperatorCode::create(
            &mut b,
            &fb::OperatorCodeArgs {
                builtin_code: op,
                deprecated_builtin_code: op.0 as i8,
                ..Default::default()
            },
        );
        let tensors = b.create_vector(&tensors);
        let operators = b.create_vector(&[node]);
        let ins = b.create_vector(&[0i32]);
        let sub = fb::SubGraph::create(
            &mut b,
            &fb::SubGraphArgs {
                tensors: Some(tensors),
                operators: Some(operators),
                inputs: Some(ins),
                outputs: Some(outs),
                ..Default::default()
            },
        );
        let subgraphs = b.create_vector(&[sub]);
        let codes = b.create_vector(&[code]);
        let buffers = b.create_vector(&buffers);
        let model = fb::Model::create(
            &mut b,
            &fb::ModelArgs {
                version: 3,
                operator_codes: Some(codes),
                subgraphs: Some(subgraphs),
                buffers: Some(buffers),
                ..Default::default()
            },
        );
        fb::finish_model_buffer(&mut b, model);
        b.finished_data().to_vec()
    }
    fn infer(bytes: Vec<u8>, shape: &[usize], values: &[f32]) -> Vec<f32> {
        let model = framework()
            .model_for_read(&mut std::io::Cursor::new(bytes))
            .unwrap()
            .into_optimized()
            .unwrap()
            .into_runnable()
            .unwrap();
        let input = Tensor::from_shape(shape, values).unwrap();
        model.run(tvec!(input.into())).unwrap()[0]
            .to_plain_array_view::<f32>()
            .unwrap()
            .iter()
            .copied()
            .collect()
    }
    #[test]
    fn float_mean_with_negative_axis_preserves_fractional_values() {
        let bytes = graph(
            BuiltinOperator::MEAN,
            &[1, 2],
            &[1],
            &[vec![-1]],
            &[vec![1]],
        );
        let value = infer(bytes, &[1, 2], &[0.1, 0.2]);
        assert_eq!(value.len(), 1);
        assert!((value[0] - 0.15).abs() < 1e-6, "{value:?}");
    }
    #[test]
    fn scalar_tensor_with_omitted_shape_imports_and_negates() {
        let bytes = graph(BuiltinOperator::NEG, &[], &[], &[], &[]);
        let value = infer(bytes, &[], &[0.2]);
        assert!((value[0] + 0.2).abs() < 1e-6);
    }
    #[test]
    fn strided_slice_honors_shrunk_axis() {
        let bytes = graph(
            BuiltinOperator::STRIDED_SLICE,
            &[2, 3],
            &[2],
            &[vec![0, 1], vec![2, 2], vec![1, 1]],
            &[vec![2], vec![2], vec![2]],
        );
        let value = infer(bytes, &[2, 3], &[1., 2., 3., 4., 5., 6.]);
        assert_eq!(value, vec![2., 5.]);
    }
}
