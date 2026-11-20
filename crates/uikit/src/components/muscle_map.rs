//! Anatomical body map (Penpot component `Mapa muscular`): one view at a
//! time, front or back (decision 4B), muscles filled by how much they were
//! trained, deficits outlined dashed, excess filled coral.

mod outlines;

use freya::engine::prelude::{Canvas, Paint, PaintStyle, Path, PathBuilder, PathEffect};
use freya::prelude::*;

use crate::theme::use_theme;
use crate::tokens::{Palette, Stroke};

/// Drawable regions of the map. The domain maps its muscles and subzones
/// onto these; [`BodyRegion::Head`] and [`BodyRegion::Knees`] are body
/// silhouette, never highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyRegion {
    /// Silhouette only.
    Head,
    /// Neck.
    Neck,
    /// Trapezius.
    Trapezius,
    /// Front (anterior) deltoids.
    FrontDeltoids,
    /// Rear (posterior) deltoids.
    BackDeltoids,
    /// Pectorals.
    Chest,
    /// Biceps.
    Biceps,
    /// Triceps.
    Triceps,
    /// Forearms.
    Forearms,
    /// Rectus abdominis.
    Abs,
    /// Obliques.
    Obliques,
    /// Lats and rhomboids.
    UpperBack,
    /// Erectors.
    LowerBack,
    /// Glutes.
    Glutes,
    /// Hip abductors.
    Abductors,
    /// Hip adductors.
    Adductors,
    /// Quadriceps.
    Quadriceps,
    /// Hamstrings.
    Hamstrings,
    /// Silhouette only.
    Knees,
    /// Gastrocnemius.
    Calves,
    /// Soleus.
    Soleus,
}

impl BodyRegion {
    /// Number of regions.
    pub const COUNT: usize = 21;

    /// Whether the region is a muscle (can be highlighted and pressed).
    #[must_use]
    pub const fn is_muscle(self) -> bool {
        !matches!(self, Self::Head | Self::Knees)
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Which side of the body is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BodyView {
    /// Anterior view.
    #[default]
    Front,
    /// Posterior view.
    Back,
}

impl BodyView {
    /// The other side, for the "Girar" button.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Front => Self::Back,
            Self::Back => Self::Front,
        }
    }

    const fn shapes(self) -> &'static [Shape] {
        match self {
            Self::Front => outlines::FRONT,
            Self::Back => outlines::BACK,
        }
    }
}

/// How one region is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RegionState {
    /// Fill step of the volume ramp, `0` (untrained) to `3` (on target).
    /// Values above 3 draw as 3.
    Load(u8),
    /// Below range: current load fill plus a dashed outline.
    Deficit(u8),
    /// Above range: coral fill.
    Excess,
    /// Nothing logged yet.
    #[default]
    Empty,
}

impl RegionState {
    const fn fill(self, palette: &Palette) -> Color {
        match self {
            Self::Load(step) | Self::Deficit(step) => {
                palette.map_levels[if step > 3 { 3 } else { step as usize }]
            }
            Self::Excess => palette.excess,
            Self::Empty => palette.map_levels[0],
        }
    }
}

/// State of every region. Unset regions are [`RegionState::Empty`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MuscleMapState([RegionState; BodyRegion::COUNT]);

impl Default for MuscleMapState {
    fn default() -> Self {
        Self([RegionState::Empty; BodyRegion::COUNT])
    }
}

impl MuscleMapState {
    /// Set `region` to `state`.
    #[must_use]
    pub const fn with(mut self, region: BodyRegion, state: RegionState) -> Self {
        self.0[region.index()] = state;
        self
    }

    /// State of `region`.
    #[must_use]
    pub const fn get(&self, region: BodyRegion) -> RegionState {
        self.0[region.index()]
    }
}

impl FromIterator<(BodyRegion, RegionState)> for MuscleMapState {
    fn from_iter<I: IntoIterator<Item = (BodyRegion, RegionState)>>(iter: I) -> Self {
        iter.into_iter()
            .fold(Self::default(), |map, (region, state)| {
                map.with(region, state)
            })
    }
}

/// One outline polygon in the 1000 × 2000 model box.
#[derive(Debug)]
struct Shape {
    region: BodyRegion,
    points: &'static [(f32, f32)],
}

impl Shape {
    /// Closed Catmull-Rom spline through the polygon corners, so the
    /// low-poly outlines read as muscle bellies.
    fn path(&self) -> Path {
        let points = self.points;
        let n = points.len();
        let at = |i: usize| points[i % n];
        let mut builder = PathBuilder::new();
        builder.move_to(at(0));
        for i in 0..n {
            let (p0, p1, p2, p3) = (at(i + n - 1), at(i), at(i + 1), at(i + 2));
            builder.cubic_to(
                (p1.0 + (p2.0 - p0.0) / 6., p1.1 + (p2.1 - p0.1) / 6.),
                (p2.0 - (p3.0 - p1.0) / 6., p2.1 - (p3.1 - p1.1) / 6.),
                p2,
            );
        }
        builder.close();
        builder.detach()
    }
}

/// Model box size.
const MODEL: (f32, f32) = (1000., 2000.);
/// Silhouette stroke in model units: thick enough to merge the polygons
/// into one body outline.
const SILHOUETTE_STROKE: f32 = 26.;

/// Uniform scale and offset that fit the model box centered in `size`.
fn fit(size: Size2D) -> (f32, (f32, f32)) {
    let scale = (size.width / MODEL.0).min(size.height / MODEL.1);
    let offset = (
        (MODEL.0.mul_add(-scale, size.width)) / 2.,
        (MODEL.1.mul_add(-scale, size.height)) / 2.,
    );
    (scale, offset)
}

/// Body map of one view. Default size follows the proposal (220 × 420);
/// set `.width(..)` / `.height(..)` to resize, the drawing keeps its aspect.
///
/// ```rust,no_run
/// # use freya::prelude::*;
/// # use myos_uikit::{BodyRegion, BodyView, MuscleMap, MuscleMapState, RegionState};
/// fn app() -> impl IntoElement {
///     let state = MuscleMapState::default()
///         .with(BodyRegion::Chest, RegionState::Load(3))
///         .with(BodyRegion::Hamstrings, RegionState::Deficit(1));
///     MuscleMap::new(BodyView::Back).state(state)
/// }
/// ```
#[derive(Clone, PartialEq)]
pub struct MuscleMap {
    view: BodyView,
    state: MuscleMapState,
    on_region_press: Option<EventHandler<BodyRegion>>,
    layout: LayoutData,
}

impl MuscleMap {
    /// Empty map of `view`.
    #[must_use]
    pub fn new(view: BodyView) -> Self {
        Self {
            view,
            state: MuscleMapState::default(),
            on_region_press: None,
            layout: LayoutData::default(),
        }
        .width(Size::px(220.))
        .height(Size::px(420.))
    }

    /// Region states.
    #[must_use]
    pub const fn state(mut self, state: MuscleMapState) -> Self {
        self.state = state;
        self
    }

    /// Called with the muscle under a click or tap.
    #[must_use]
    pub fn on_region_press(mut self, on_region_press: impl Into<EventHandler<BodyRegion>>) -> Self {
        self.on_region_press = Some(on_region_press.into());
        self
    }
}

impl LayoutExt for MuscleMap {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for MuscleMap {}

/// Paint the body: silhouette, muscle fills, separators, deficit outlines.
fn draw(canvas: &Canvas, size: Size2D, view: BodyView, state: MuscleMapState, palette: &Palette) {
    let (scale, (dx, dy)) = fit(size);
    canvas.save();
    // The canvas arrives translated to the element origin, in logical px.
    canvas.translate((dx, dy));
    canvas.scale((scale, scale));

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let shapes: Vec<(BodyRegion, Path)> = view
        .shapes()
        .iter()
        .map(|shape| (shape.region, shape.path()))
        .collect();

    paint.set_color(palette.map_silhouette);
    paint.set_style(PaintStyle::StrokeAndFill);
    paint.set_stroke_width(SILHOUETTE_STROKE);
    for (_, path) in &shapes {
        canvas.draw_path(path, &paint);
    }

    let separator = Stroke::HAIRLINE / scale;
    for (region, path) in shapes.iter().filter(|(region, _)| region.is_muscle()) {
        let region_state = state.get(*region);
        paint.set_style(PaintStyle::Fill);
        paint.set_path_effect(None);
        paint.set_color(region_state.fill(palette));
        canvas.draw_path(path, &paint);

        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(separator);
        paint.set_color(palette.ground);
        canvas.draw_path(path, &paint);

        if let RegionState::Deficit(_) = region_state {
            paint.set_stroke_width(Stroke::REGULAR / scale);
            paint.set_color(palette.deficit);
            paint.set_path_effect(PathEffect::dash(&[3. / scale, 3. / scale], 0.));
            canvas.draw_path(path, &paint);
        }
    }
    canvas.restore();
}

impl Component for MuscleMap {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let view = self.view;
        let state = self.state;
        let painter = RenderCallback::new(move |ctx: &mut FillRenderContext| {
            if let RenderTarget::Background { canvas, .. } = &ctx.target {
                draw(canvas, ctx.size, view, state, palette);
            }
        });
        let mut size = use_state(Size2D::default);
        rect()
            .layout(self.layout.clone())
            .background(painter)
            .on_sized(move |e: Event<SizedEventData>| size.set_if_modified(e.area.size))
            .a11y_role(AccessibilityRole::Image)
            .maybe(self.on_region_press.is_some(), |el| {
                el.cursor(CursorIcon::Pointer)
            })
            .map(self.on_region_press.clone(), |el, handler| {
                el.on_press(move |e: Event<PressEventData>| {
                    let location = match e.data() {
                        PressEventData::Mouse(data) => data.element_location,
                        PressEventData::Touch(data) => data.element_location,
                        PressEventData::Keyboard(_) | PressEventData::Accessibility(_) => return,
                    };
                    let (scale, (dx, dy)) = fit(*size.read());
                    #[expect(clippy::cast_possible_truncation, reason = "logical px fit in f32")]
                    let point = (
                        ((location.x as f32) - dx) / scale,
                        ((location.y as f32) - dy) / scale,
                    );
                    let hit = view
                        .shapes()
                        .iter()
                        .rev()
                        .find(|shape| shape.region.is_muscle() && shape.path().contains(point));
                    if let Some(shape) = hit {
                        handler.call(shape.region);
                    }
                })
            })
    }
}
