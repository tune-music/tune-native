//! Живой фон плеера на весь экран: мягкие пятна в цветах обложки медленно текут и дышат
//! под музыку — басы раздувают узор, середина подсвечивает пятна, верха добавляют яркости.
//! В облегчённой версии вместо него неподвижный градиент.

use iced::widget::shader::{self, Viewport};
use iced::{mouse, wgpu, Color, Rectangle};

/// Что рисовать: основной цвет, время анимации и уровни басов, середины и верхов от 0 до 1.
#[derive(Clone, Copy, Debug)]
pub struct Vibe {
    pub color: Color,
    pub time: f32,
    pub levels: [f32; 3],
}

impl<Message> shader::Program<Message> for Vibe {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Primitive {
        Primitive(*self)
    }
}

#[derive(Debug)]
pub struct Primitive(Vibe);

/// Пять `vec4` по порядку полей в шейдере.
const UNIFORMS: u64 = 5 * 16;

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// Кадр в sRGB: цвета надо отдать линейными, перевод сделает видеокарта.
    srgb: bool,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("vibe"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor { label: Some("vibe"), size: UNIFORMS, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibe"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("vibe"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }] });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("vibe"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, uniforms, bind, srgb: format.is_srgb() }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(&self, pipeline: &mut Pipeline, _device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let Vibe { color, time, levels } = self.0;
        let shade = |level: f32| {
            let c = Color::from_rgb((color.r * level).min(1.0), (color.g * level).min(1.0), (color.b * level).min(1.0));
            let c = if pipeline.srgb { c.into_linear() } else { [c.r, c.g, c.b, 1.0] };
            [c[0], c[1], c[2], 1.0]
        };
        let scale = viewport.scale_factor();
        let values: [[f32; 4]; 5] = [
            shade(BASE),
            shade(BASE * 1.7),
            shade(BASE * 0.3),
            [bounds.x * scale, bounds.y * scale, bounds.width * scale, bounds.height * scale],
            [time, levels[0], levels[1], levels[2]],
        ];
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|x| x.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, 0, &bytes);
    }

    fn draw(&self, pipeline: &Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &pipeline.bind, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

/// Насколько приглушён цвет обложки: как верх прежнего градиента, чтобы белый текст читался.
const BASE: f32 = 0.55;

const SHADER: &str = r#"
struct Uniforms {
    base: vec4<f32>,
    light: vec4<f32>,
    deep: vec4<f32>,
    area: vec4<f32>,
    motion: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

// Один треугольник на весь экран, лишнее отрезает рамка виджета.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

/// Мягкое пятно: гауссов спад от центра.
fn blob(p: vec2<f32>, center: vec2<f32>, radius: f32) -> f32 {
    let d = p - center;
    return exp(-dot(d, d) / (radius * radius));
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = u.area.zw;
    let local = pos.xy - u.area.xy;
    let aspect = size.x / size.y;
    // Масштаб по высоте: пятна одной величины в любом окне.
    var p = local / size.y;
    let t = u.motion.x;
    let low = u.motion.y;
    let mid = u.motion.z;
    let high = u.motion.w;
    // Лёгкий вихрь, чтобы края пятен текли, а не были кругами.
    p += 0.06 * vec2<f32>(sin(p.y * 3.1 + t * 0.31), cos(p.x * 2.7 - t * 0.27));
    let grow = 1.0 + low * 0.35;
    let glow = 0.55 + mid * 0.6;
    // Пять пятен ходят каждое по своей петле с несоизмеримыми скоростями: узор не повторяется.
    let c1 = vec2<f32>(aspect * (0.30 + 0.20 * sin(t * 0.130)), 0.30 + 0.18 * cos(t * 0.170));
    let c2 = vec2<f32>(aspect * (0.70 + 0.18 * cos(t * 0.110)), 0.35 + 0.20 * sin(t * 0.150 + 1.0));
    let c3 = vec2<f32>(aspect * (0.50 + 0.30 * sin(t * 0.070 + 2.0)), 0.75 + 0.15 * cos(t * 0.090));
    let c4 = vec2<f32>(aspect * (0.20 + 0.15 * cos(t * 0.190 + 0.5)), 0.70 + 0.20 * sin(t * 0.120 + 3.0));
    let c5 = vec2<f32>(aspect * (0.85 + 0.10 * sin(t * 0.160 + 4.0)), 0.80 + 0.15 * cos(t * 0.140 + 2.0));
    let w1 = blob(p, c1, 0.42 * grow);
    let w2 = blob(p, c2, 0.36 * grow) * glow;
    let w3 = blob(p, c3, 0.48 * grow);
    let w4 = blob(p, c4, 0.30 * grow) * glow;
    let w5 = blob(p, c5, 0.34 * grow);
    let light = u.light.rgb * (1.0 + high * 0.25);
    let sum = w1 * u.base.rgb + w2 * light + w3 * u.base.rgb * 0.8 + w4 * light * 0.9 + w5 * u.deep.rgb * 2.0;
    let weight = w1 + w2 + w3 + w4 + w5;
    var color = mix(u.deep.rgb, sum / max(weight, 0.0001), clamp(weight, 0.0, 1.0));
    // Сверху светлее, к низу темнее: снизу кнопки и громкость.
    color *= mix(1.0, 0.45, smoothstep(0.1, 1.0, local.y / size.y));
    // Лёгкий шум против полос на плавных переходах.
    color += (hash(pos.xy) - 0.5) / 255.0;
    return vec4<f32>(color, 1.0);
}
"#;
