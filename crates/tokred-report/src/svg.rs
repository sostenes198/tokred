//! SVG chart generators: lines, vertical and horizontal bars, and donuts.
//!
//! Every function returns a self-contained `<svg>` fragment (no `xmlns`, no external
//! references) with a `<title>`, `role="img"` and `aria-labelledby`. Colours come from
//! CSS classes (`s0`..`s5`) so the light and dark themes can restyle them; hover text
//! lives in `data-tip` attributes read by the inline script. Data values are
//! sanitized: negative and non-finite numbers are drawn as zero, so no coordinate is
//! ever negative or `NaN`.

use crate::escape;

/// Width of the line and vertical-bar viewBox.
const WIDTH: f64 = 640.0;
/// Height of the line and vertical-bar viewBox.
const HEIGHT: f64 = 240.0;
/// Maximum number of x-axis labels shown.
const MAX_X_LABELS: usize = 6;
/// Maximum characters of a bar label before it is truncated.
const MAX_LABEL_CHARS: usize = 24;

/// A value formatter for axis ticks and tooltips.
pub type ValueFormat = fn(f64) -> String;

/// Clamps a data value: non-finite or negative becomes `0.0`.
fn sanitize(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// Formats a coordinate with one decimal.
fn coord(value: f64) -> String {
    format!("{:.1}", sanitize(value))
}

/// Converts an index to `f64` without a lossy cast.
pub(crate) fn index_f64(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

/// Rounds `max` up to a "nice" axis maximum (1, 2, 2.5, 5 or 10 times a power of ten).
///
/// Non-positive or non-finite input gives `1.0`.
#[must_use]
pub fn nice_ceil(max: f64) -> f64 {
    if !max.is_finite() || max <= 0.0 {
        return 1.0;
    }
    let magnitude = 10_f64.powf(max.log10().floor());
    let ratio = max / magnitude;
    let step = [1.0, 2.0, 2.5, 5.0]
        .into_iter()
        .find(|candidate| ratio <= *candidate + 1e-9)
        .unwrap_or(10.0);
    step * magnitude
}

/// Picks the axis maximum: the fixed one when valid, otherwise a nice ceiling of the data.
fn axis_max(fixed: Option<f64>, data_max: f64) -> f64 {
    match fixed {
        Some(max) if max.is_finite() && max > 0.0 => max,
        _ => nice_ceil(data_max),
    }
}

/// Truncates a label to [`MAX_LABEL_CHARS`] characters.
fn truncate(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        label.to_owned()
    } else {
        let head: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
        format!("{head}…")
    }
}

/// The drawable rectangle of a chart.
struct Plot {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

impl Plot {
    /// The plot area shared by the line and vertical-bar charts.
    const fn standard() -> Self {
        Self {
            left: 56.0,
            top: 12.0,
            width: WIDTH - 56.0 - 12.0,
            height: HEIGHT - 12.0 - 30.0,
        }
    }

    /// X of point `index` out of `count` (a single point sits in the middle).
    fn x_at(&self, index: usize, count: usize) -> f64 {
        if count < 2 {
            self.left + self.width / 2.0
        } else {
            self.left + self.width * index_f64(index) / index_f64(count - 1)
        }
    }

    /// Y of `value` on an axis that goes from 0 to `max`; values above `max` are clamped.
    fn y_at(&self, value: f64, max: f64) -> f64 {
        let ratio = (sanitize(value) / max).min(1.0);
        self.top + self.height * (1.0 - ratio)
    }

    /// Horizontal grid lines with their tick labels.
    fn grid(&self, max: f64, format: ValueFormat) -> String {
        let mut out = String::new();
        for step in 0..=4_u32 {
            let fraction = f64::from(step) / 4.0;
            let y = coord(self.top + self.height * (1.0 - fraction));
            out.push_str(&format!(
                "<line class=\"grid\" x1=\"{}\" y1=\"{y}\" x2=\"{}\" y2=\"{y}\"/>\
                 <text class=\"tick\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text>",
                coord(self.left),
                coord(self.left + self.width),
                coord(self.left - 6.0),
                coord(self.top + self.height * (1.0 - fraction) + 4.0),
                escape::html(&format(max * fraction)),
            ));
        }
        out
    }

    /// X-axis labels, thinned to at most [`MAX_X_LABELS`] and centred under each position.
    fn x_labels(&self, labels: &[String], centred_in_slot: bool) -> String {
        let count = labels.len();
        let stride = count.div_ceil(MAX_X_LABELS).max(1);
        let mut out = String::new();
        for (index, label) in labels.iter().enumerate().step_by(stride) {
            let x = if centred_in_slot {
                self.left + self.width * (index_f64(index) + 0.5) / index_f64(count)
            } else {
                self.x_at(index, count)
            };
            out.push_str(&format!(
                "<text class=\"tick\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
                coord(x),
                coord(self.top + self.height + 18.0),
                escape::html(label),
            ));
        }
        out
    }
}

/// Opens an `<svg>` with accessible title.
fn open(id: &str, title: &str, width: f64, height: f64) -> String {
    format!(
        "<svg class=\"chart\" viewBox=\"0 0 {} {}\" role=\"img\" aria-labelledby=\"{id}-title\">\
         <title id=\"{id}-title\">{}</title>",
        coord(width),
        coord(height),
        escape::html(title),
    )
}

/// The "no data" placeholder drawn inside an otherwise empty chart.
fn no_data(plot: &Plot) -> String {
    format!(
        "<text class=\"nodata\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">No data</text>",
        coord(plot.left + plot.width / 2.0),
        coord(plot.top + plot.height / 2.0),
    )
}

/// One line of a [`LineSpec`].
pub struct Series<'a> {
    /// Series name (used in tooltips).
    pub name: &'a str,
    /// One value per x label.
    pub values: &'a [f64],
    /// Colour class index (`s0`..`s5`).
    pub style: usize,
    /// Whether the line is dashed (so colour is not the only encoding).
    pub dashed: bool,
}

/// Input of [`line_chart`].
pub struct LineSpec<'a> {
    /// Unique id prefix (the title id derives from it).
    pub id: &'a str,
    /// Accessible title.
    pub title: &'a str,
    /// X labels, one per point.
    pub labels: &'a [String],
    /// The lines.
    pub series: &'a [Series<'a>],
    /// Fixed axis maximum; `None` derives a nice maximum from the data.
    pub y_max: Option<f64>,
    /// Formatter for ticks and tooltips.
    pub format: ValueFormat,
}

/// Draws a line chart with one marker per point.
#[must_use]
pub fn line_chart(spec: &LineSpec<'_>) -> String {
    let plot = Plot::standard();
    let count = spec.labels.len();
    let data_max = spec
        .series
        .iter()
        .flat_map(|series| series.values.iter())
        .fold(0.0_f64, |max, value| max.max(sanitize(*value)));
    let max = axis_max(spec.y_max, data_max);
    let radius = if count > 31 { "2" } else { "3" };

    let mut out = open(spec.id, spec.title, WIDTH, HEIGHT);
    out.push_str(&plot.grid(max, spec.format));
    out.push_str(&plot.x_labels(spec.labels, false));
    if count == 0 {
        out.push_str(&no_data(&plot));
    }
    for series in spec.series {
        let points: Vec<(f64, f64)> = series
            .values
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, value)| (plot.x_at(index, count), plot.y_at(*value, max)))
            .collect();
        let dash = if series.dashed { " dash" } else { "" };
        let polyline: Vec<String> = points
            .iter()
            .map(|(x, y)| format!("{},{}", coord(*x), coord(*y)))
            .collect();
        out.push_str(&format!(
            "<polyline class=\"line s{}{dash}\" points=\"{}\"/>",
            series.style,
            polyline.join(" "),
        ));
        for (index, ((x, y), value)) in points.iter().zip(series.values).enumerate() {
            let label = spec.labels.get(index).map_or("", String::as_str);
            out.push_str(&format!(
                "<circle class=\"dot s{}\" cx=\"{}\" cy=\"{}\" r=\"{radius}\" tabindex=\"0\" \
                 data-tip=\"{}\"/>",
                series.style,
                coord(*x),
                coord(*y),
                escape::html(&format!(
                    "{label}: {} {}",
                    series.name,
                    (spec.format)(sanitize(*value))
                )),
            ));
        }
    }
    out.push_str("</svg>");
    out
}

/// Input of [`vbar_chart`].
pub struct VBarSpec<'a> {
    /// Unique id prefix (the title id derives from it).
    pub id: &'a str,
    /// Accessible title.
    pub title: &'a str,
    /// X labels, one per bar.
    pub labels: &'a [String],
    /// One value per bar.
    pub values: &'a [f64],
    /// Colour class index.
    pub style: usize,
    /// Fixed axis maximum; `None` derives a nice maximum from the data.
    pub y_max: Option<f64>,
    /// Formatter for ticks and tooltips.
    pub format: ValueFormat,
}

/// Draws a vertical bar chart (one bar per label).
#[must_use]
pub fn vbar_chart(spec: &VBarSpec<'_>) -> String {
    let plot = Plot::standard();
    let count = spec.labels.len();
    let data_max = spec
        .values
        .iter()
        .fold(0.0_f64, |max, v| max.max(sanitize(*v)));
    let max = axis_max(spec.y_max, data_max);

    let mut out = open(spec.id, spec.title, WIDTH, HEIGHT);
    out.push_str(&plot.grid(max, spec.format));
    out.push_str(&plot.x_labels(spec.labels, true));
    if count == 0 {
        out.push_str(&no_data(&plot));
    }
    let slot = plot.width / index_f64(count.max(1));
    let bar = (slot * 0.7).max(1.0);
    let baseline = plot.top + plot.height;
    for (index, (label, value)) in spec.labels.iter().zip(spec.values).enumerate() {
        let y = plot.y_at(*value, max);
        let x = plot.left + slot * index_f64(index) + (slot - bar) / 2.0;
        out.push_str(&format!(
            "<rect class=\"bar s{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
             tabindex=\"0\" data-tip=\"{}\"/>",
            spec.style,
            coord(x),
            coord(y),
            coord(bar),
            coord(baseline - y),
            escape::html(&format!("{label}: {}", (spec.format)(sanitize(*value)))),
        ));
    }
    out.push_str("</svg>");
    out
}

/// A vertical reference line of a [`HBarSpec`] (for example the latency budget).
pub struct Reference<'a> {
    /// Where the line is drawn, in data units.
    pub value: f64,
    /// Text drawn next to the line.
    pub label: &'a str,
}

/// Input of [`hbar_chart`].
pub struct HBarSpec<'a> {
    /// Unique id prefix (the title id derives from it).
    pub id: &'a str,
    /// Accessible title.
    pub title: &'a str,
    /// Names of the bars inside each group, with their colour class index.
    pub series: &'a [(&'a str, usize)],
    /// Groups: a label and one value per series.
    pub groups: &'a [(String, Vec<f64>)],
    /// Formatter for value labels and tooltips.
    pub format: ValueFormat,
    /// Optional reference line.
    pub reference: Option<Reference<'a>>,
}

/// Draws horizontal grouped bars, each with a text value label.
///
/// With more than one series the label is prefixed with the series name, so colour
/// is never the only way to tell the bars apart.
#[must_use]
pub fn hbar_chart(spec: &HBarSpec<'_>) -> String {
    const LABEL_COLUMN: f64 = 150.0;
    const VALUE_COLUMN: f64 = 96.0;
    const BAR_HEIGHT: f64 = 14.0;
    const GROUP_GAP: f64 = 10.0;
    const HEADER: f64 = 18.0;

    let series_count = spec.series.len().max(1);
    let group_height = index_f64(series_count) * BAR_HEIGHT + GROUP_GAP;
    let height = HEADER + index_f64(spec.groups.len().max(1)) * group_height + 6.0;
    let plot_width = WIDTH - LABEL_COLUMN - VALUE_COLUMN;
    let reference_value = spec.reference.as_ref().map_or(0.0, |r| sanitize(r.value));
    let data_max = spec
        .groups
        .iter()
        .flat_map(|(_, values)| values.iter())
        .fold(reference_value, |max, v| max.max(sanitize(*v)));
    let max = nice_ceil(data_max);
    let scale = |value: f64| plot_width * (sanitize(value) / max).min(1.0);

    let mut out = open(spec.id, spec.title, WIDTH, height);
    if spec.groups.is_empty() {
        out.push_str(&format!(
            "<text class=\"nodata\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">No data</text>",
            coord(WIDTH / 2.0),
            coord(height / 2.0),
        ));
    }
    if let Some(reference) = &spec.reference {
        let x = coord(LABEL_COLUMN + scale(reference.value));
        out.push_str(&format!(
            "<line class=\"ref\" x1=\"{x}\" y1=\"{}\" x2=\"{x}\" y2=\"{}\"/>\
             <text class=\"tick\" x=\"{x}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            coord(HEADER),
            coord(height - 4.0),
            coord(HEADER - 6.0),
            escape::html(reference.label),
        ));
    }
    for (group_index, (label, values)) in spec.groups.iter().enumerate() {
        let group_top = HEADER + index_f64(group_index) * group_height;
        out.push_str(&format!(
            "<text class=\"label\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text>",
            coord(LABEL_COLUMN - 8.0),
            coord(group_top + BAR_HEIGHT),
            escape::html(&truncate(label)),
        ));
        for (series_index, ((name, style), value)) in spec.series.iter().zip(values).enumerate() {
            let y = group_top + index_f64(series_index) * BAR_HEIGHT;
            let width = scale(*value);
            let text = if spec.series.len() > 1 {
                format!("{name} {}", (spec.format)(sanitize(*value)))
            } else {
                (spec.format)(sanitize(*value))
            };
            out.push_str(&format!(
                "<rect class=\"bar s{style}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 tabindex=\"0\" data-tip=\"{}\"/>\
                 <text class=\"val\" x=\"{}\" y=\"{}\">{}</text>",
                coord(LABEL_COLUMN),
                coord(y),
                coord(width),
                coord(BAR_HEIGHT - 2.0),
                escape::html(&format!(
                    "{label}: {name} {}",
                    (spec.format)(sanitize(*value))
                )),
                coord(LABEL_COLUMN + width + 6.0),
                coord(y + BAR_HEIGHT - 4.0),
                escape::html(&text),
            ));
        }
    }
    out.push_str("</svg>");
    out
}

/// One slice of a [`donut`].
pub struct Slice {
    /// Slice label (used in the tooltip).
    pub label: String,
    /// Slice value; non-positive values are skipped.
    pub value: f64,
    /// Colour class index.
    pub style: usize,
    /// Text for the tooltip value (already formatted).
    pub display: String,
}

/// Draws a donut chart; `center` is the text in the hole (for example the total).
///
/// Slices are `<circle>` strokes with a dash pattern, so a single slice of 100% is a
/// full ring and there are no degenerate arcs. A zero total draws an empty ring.
#[must_use]
pub fn donut(id: &str, title: &str, slices: &[Slice], center: &str) -> String {
    const SIZE: f64 = 200.0;
    const RADIUS: f64 = 70.0;
    const STROKE: f64 = 28.0;
    let circumference = 2.0 * std::f64::consts::PI * RADIUS;
    let middle = SIZE / 2.0;
    let total: f64 = slices.iter().map(|slice| sanitize(slice.value)).sum();

    let mut out = format!(
        "<svg class=\"chart donut\" viewBox=\"0 0 {} {}\" role=\"img\" \
         aria-labelledby=\"{id}-title\"><title id=\"{id}-title\">{}</title>",
        coord(SIZE),
        coord(SIZE),
        escape::html(title),
    );
    out.push_str(&format!(
        "<circle class=\"ring\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke-width=\"{}\"/>",
        coord(middle),
        coord(middle),
        coord(RADIUS),
        coord(STROKE),
    ));
    let mut offset = 0.0_f64;
    for slice in slices.iter().filter(|slice| sanitize(slice.value) > 0.0) {
        let length = circumference * sanitize(slice.value) / total;
        out.push_str(&format!(
            "<circle class=\"slice s{}\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" \
             stroke-width=\"{}\" stroke-dasharray=\"{:.2} {:.2}\" stroke-dashoffset=\"{:.2}\" \
             transform=\"rotate(-90 {} {})\" tabindex=\"0\" data-tip=\"{}\"/>",
            slice.style,
            coord(middle),
            coord(middle),
            coord(RADIUS),
            coord(STROKE),
            length,
            (circumference - length).max(0.0),
            sanitize(circumference - offset),
            coord(middle),
            coord(middle),
            escape::html(&format!("{}: {}", slice.label, slice.display)),
        ));
        offset += length;
    }
    out.push_str(&format!(
        "<text class=\"center\" x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text></svg>",
        coord(middle),
        coord(middle + 5.0),
        escape::html(center),
    ));
    out
}

/// Draws a legend (`<ul>`) with a swatch per entry: `(label, colour index, dashed)`.
#[must_use]
pub fn legend(entries: &[(&str, usize, bool)]) -> String {
    let mut out = String::from("<ul class=\"legend\">");
    for (label, style, dashed) in entries {
        let dash = if *dashed { " dash" } else { "" };
        out.push_str(&format!(
            "<li><span class=\"sw s{style}{dash}\" aria-hidden=\"true\"></span>{}</li>",
            escape::html(label),
        ));
    }
    out.push_str("</ul>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::assert_well_formed;

    fn labels(n: usize) -> Vec<String> {
        (1..=n).map(|day| format!("2026-10-{day:02}")).collect()
    }

    fn money(value: f64) -> String {
        crate::format::usd(value)
    }

    fn line(labels: &[String], values: &[f64], y_max: Option<f64>) -> String {
        let series = [Series {
            name: "Real",
            values,
            style: 4,
            dashed: false,
        }];
        line_chart(&LineSpec {
            id: "c",
            title: "Cost",
            labels,
            series: &series,
            y_max,
            format: money,
        })
    }

    /// No coordinate attribute may start with a minus sign or be `NaN`/`inf`.
    fn assert_sane_numbers(svg: &str) {
        for needle in ["=\"-", " -", "NaN", "inf"] {
            assert!(!svg.contains(needle), "found `{needle}` in {svg}");
        }
    }

    #[test]
    fn nice_ceil_rounds_up_to_round_numbers() {
        assert!((nice_ceil(0.0) - 1.0).abs() < 1e-9);
        assert!((nice_ceil(-4.0) - 1.0).abs() < 1e-9);
        assert!((nice_ceil(f64::NAN) - 1.0).abs() < 1e-9);
        assert!((nice_ceil(0.3) - 0.5).abs() < 1e-9);
        assert!((nice_ceil(1.0) - 1.0).abs() < 1e-9);
        assert!((nice_ceil(1.7) - 2.0).abs() < 1e-9);
        assert!((nice_ceil(2.2) - 2.5).abs() < 1e-9);
        assert!((nice_ceil(3.0) - 5.0).abs() < 1e-9);
        assert!((nice_ceil(7.0) - 10.0).abs() < 1e-9);
        assert!((nice_ceil(1234.0) - 2000.0).abs() < 1e-6);
        assert!(nice_ceil(1e300) >= 1e300);
    }

    #[test]
    fn sanitize_clamps_bad_values() {
        assert!(sanitize(-1.0).abs() < f64::EPSILON);
        assert!(sanitize(f64::NAN).abs() < f64::EPSILON);
        assert!(sanitize(f64::INFINITY).abs() < f64::EPSILON);
        assert!((sanitize(2.5) - 2.5).abs() < f64::EPSILON);
        assert_eq!(coord(-0.0), "0.0");
    }

    #[test]
    fn index_f64_saturates() {
        assert!((index_f64(3) - 3.0).abs() < f64::EPSILON);
        assert!(index_f64(usize::MAX) > 4e9);
    }

    #[test]
    fn truncate_shortens_long_labels() {
        assert_eq!(truncate("short"), "short");
        let long = "x".repeat(40);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_LABEL_CHARS);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn line_chart_with_no_points_says_so() {
        let svg = line(&[], &[], None);
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("No data"));
        assert!(svg.contains("points=\"\""));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_with_one_point_draws_a_marker_in_the_middle() {
        let svg = line(&labels(1), &[2.0], None);
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("<circle class=\"dot s4\" cx=\"342.0\""));
        assert!(!svg.contains("No data"));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_draws_dashed_series_and_tooltips() {
        let first = [1.0, 2.0, 3.0];
        let second = [2.0, 4.0, 6.0];
        let series = [
            Series {
                name: "Real",
                values: &first,
                style: 4,
                dashed: false,
            },
            Series {
                name: "All opus",
                values: &second,
                style: 3,
                dashed: true,
            },
        ];
        let svg = line_chart(&LineSpec {
            id: "two",
            title: "Cost <per> day",
            labels: &labels(3),
            series: &series,
            y_max: None,
            format: money,
        });
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("line s3 dash"));
        assert!(svg.contains("line s4\""));
        assert!(svg.contains("2026-10-02: All opus $4.00"));
        assert!(svg.contains("Cost &lt;per&gt; day"));
        assert!(svg.contains("aria-labelledby=\"two-title\""));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_survives_hostile_values() {
        let values = [-5.0, f64::NAN, f64::INFINITY, 1e300, 0.0];
        let svg = line(&labels(5), &values, None);
        assert_well_formed(&svg).unwrap();
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_with_zero_totals_and_dense_points() {
        let values = vec![0.0; 40];
        let svg = line(
            &labels(31).into_iter().cycle().take(40).collect::<Vec<_>>(),
            &values,
            None,
        );
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("r=\"2\""));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_clamps_values_above_a_fixed_axis() {
        let svg = line(&labels(2), &[0.5, 3.0], Some(1.0));
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("cy=\"12.0\""));
    }

    #[test]
    fn line_chart_tolerates_more_values_than_labels() {
        let svg = line(&labels(1), &[1.0, 2.0], None);
        assert_well_formed(&svg).unwrap();
        assert_sane_numbers(&svg);
    }

    #[test]
    fn line_chart_thins_x_labels() {
        let svg = line(&labels(30), &[1.0; 30], None);
        assert_eq!(svg.matches("text-anchor=\"middle\"").count(), 6);
    }

    #[test]
    fn axis_max_prefers_a_valid_fixed_maximum() {
        assert!((axis_max(Some(1.0), 50.0) - 1.0).abs() < f64::EPSILON);
        assert!((axis_max(Some(-1.0), 3.0) - 5.0).abs() < f64::EPSILON);
        assert!((axis_max(None, 3.0) - 5.0).abs() < f64::EPSILON);
    }

    fn vbars(labels: &[String], values: &[f64]) -> String {
        vbar_chart(&VBarSpec {
            id: "v",
            title: "Hit rate",
            labels,
            values,
            style: 2,
            y_max: Some(1.0),
            format: crate::format::percent,
        })
    }

    #[test]
    fn vbar_chart_draws_one_bar_per_label() {
        let svg = vbars(&labels(3), &[0.2, 0.9, 1.5]);
        assert_well_formed(&svg).unwrap();
        assert_eq!(svg.matches("<rect").count(), 3);
        assert!(svg.contains("2026-10-02: 90.0%"));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn vbar_chart_handles_empty_and_zero_input() {
        let empty = vbars(&[], &[]);
        assert_well_formed(&empty).unwrap();
        assert!(empty.contains("No data"));
        let zero = vbars(&labels(2), &[0.0, -3.0]);
        assert_well_formed(&zero).unwrap();
        assert!(zero.contains("height=\"0.0\""));
        assert_sane_numbers(&zero);
    }

    #[test]
    fn vbar_chart_derives_its_maximum_when_not_fixed() {
        let svg = vbar_chart(&VBarSpec {
            id: "v",
            title: "Cost",
            labels: &labels(2),
            values: &[1.0, 3.0],
            style: 0,
            y_max: None,
            format: money,
        });
        assert!(svg.contains("$5.00"));
    }

    fn hbars(groups: &[(String, Vec<f64>)], reference: Option<Reference<'_>>) -> String {
        hbar_chart(&HBarSpec {
            id: "h",
            title: "Latency",
            series: &[("p50", 2), ("p95", 1), ("max", 4)],
            groups,
            format: crate::format::ms,
            reference,
        })
    }

    #[test]
    fn hbar_chart_labels_every_bar_with_text() {
        let groups = vec![
            ("guard-read".to_owned(), vec![3.0, 12.0, 80.0]),
            ("x".repeat(40), vec![0.0, 0.0, 0.0]),
        ];
        let svg = hbars(
            &groups,
            Some(Reference {
                value: 50.0,
                label: "50 ms budget",
            }),
        );
        assert_well_formed(&svg).unwrap();
        assert_eq!(svg.matches("<rect").count(), 6);
        assert!(svg.contains(">p95 12.0 ms<"));
        assert!(svg.contains("50 ms budget"));
        assert!(svg.contains("class=\"ref\""));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn hbar_chart_with_a_single_series_shows_plain_values() {
        let svg = hbar_chart(&HBarSpec {
            id: "r",
            title: "Routing",
            series: &[("count", 1)],
            groups: &[("explore".to_owned(), vec![5.0])],
            format: |v| format!("{v:.0}"),
            reference: None,
        });
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains(">5<"));
        assert!(!svg.contains("class=\"ref\""));
    }

    #[test]
    fn hbar_chart_without_groups_says_no_data() {
        let svg = hbars(&[], None);
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("No data"));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn hbar_chart_survives_huge_and_negative_values() {
        let groups = vec![("a".to_owned(), vec![1e300, -1.0, f64::NAN])];
        let svg = hbars(&groups, None);
        assert_well_formed(&svg).unwrap();
        assert_sane_numbers(&svg);
    }

    fn slice(label: &str, value: f64, style: usize) -> Slice {
        Slice {
            label: label.to_owned(),
            value,
            style,
            display: format!("{value}"),
        }
    }

    #[test]
    fn donut_splits_the_ring_proportionally() {
        let svg = donut(
            "d",
            "Cost by model",
            &[
                slice("opus", 1.0, 0),
                slice("sonnet", 3.0, 1),
                slice("skip", 0.0, 2),
            ],
            "$4.00",
        );
        assert_well_formed(&svg).unwrap();
        assert_eq!(svg.matches("class=\"slice").count(), 2);
        assert!(svg.contains("sonnet: 3"));
        assert!(svg.contains("$4.00"));
        assert_sane_numbers(&svg);
    }

    #[test]
    fn donut_with_one_slice_is_a_full_ring() {
        let svg = donut("d", "t", &[slice("only", 7.0, 0)], "7");
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("stroke-dasharray=\"439.82 0.00\""));
    }

    #[test]
    fn donut_with_zero_total_is_an_empty_ring() {
        let svg = donut("d", "t", &[slice("a", 0.0, 0), slice("b", -2.0, 1)], "none");
        assert_well_formed(&svg).unwrap();
        assert!(svg.contains("class=\"ring\""));
        assert!(!svg.contains("class=\"slice"));
        assert_sane_numbers(&svg);
        let nothing = donut("d", "t", &[], "none");
        assert_well_formed(&nothing).unwrap();
    }

    #[test]
    fn donut_survives_very_large_values() {
        let svg = donut(
            "d",
            "t",
            &[slice("a", 1e300, 0), slice("b", 1e300, 1)],
            "big",
        );
        assert_well_formed(&svg).unwrap();
        assert_sane_numbers(&svg);
    }

    #[test]
    fn legend_marks_dashed_entries_and_escapes_labels() {
        let html = legend(&[("Real", 4, false), ("<b>", 3, true)]);
        assert_well_formed(&html).unwrap();
        assert!(html.contains("sw s3 dash"));
        assert!(html.contains("&lt;b&gt;"));
    }
}
