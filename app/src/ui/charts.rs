use egui::{pos2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use super::theme as t;

/// 图表上的阶段分界（例如加速结束、匀速结束）。
/// `label` 为空时只画虚线，用于综合曲线之间的周期分界。
pub struct ChartMark {
    pub time: f64,
    pub label: String,
}

/// 折线图：自适应刻度、网格、零线、面积填充与阶段分界。数据为 `(时间, 数值)`。
pub fn line_chart(
    ui: &mut Ui,
    unit: &str,
    samples: &[(f64, f64)],
    color: Color32,
    marks: &[ChartMark],
) {
    let height = ui.available_height().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, egui::CornerRadius::same(10), t::SURFACE_ALT);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(10),
        Stroke::new(1.0, t::BORDER),
        egui::StrokeKind::Inside,
    );

    if samples.len() < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "等待有效工况",
            FontId::proportional(11.5),
            t::MUTED,
        );
        return;
    }

    let plot = Rect::from_min_max(
        pos2(rect.left() + 54.0, rect.top() + 18.0),
        pos2(rect.right() - 12.0, rect.bottom() - 24.0),
    );
    if plot.width() <= 4.0 || plot.height() <= 4.0 {
        return;
    }

    let t_max = samples
        .iter()
        .map(|(x, _)| *x)
        .fold(0.0, f64::max)
        .max(1e-9);

    // 纵轴范围始终包含 0，便于正负力对比。
    let mut data_min = 0.0_f64;
    let mut data_max = 0.0_f64;
    for (_, y) in samples {
        data_min = data_min.min(*y);
        data_max = data_max.max(*y);
    }
    let (y_min, y_max, y_step) = nice_range(data_min, data_max);
    let x_of = |x: f64| plot.left() + (x / t_max) as f32 * plot.width();
    let y_of = |y: f64| plot.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * plot.height();

    // 网格与刻度：横纵轴都标注数值，避免只显示两端而无法读数。
    // 网格线用比边框更淡的颜色，避免和折线抢视觉。
    let grid = Stroke::new(1.0, t::tint(t::SURFACE_ALT, t::BORDER, 0.55));
    let mut value = y_min;
    while value <= y_max + y_step * 0.5 {
        let y = y_of(value);
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], grid);
        // 左侧小刻度，方便从数值对齐到网格线。
        painter.line_segment(
            [pos2(plot.left() - 4.0, y), pos2(plot.left(), y)],
            Stroke::new(1.0, t::BORDER_STRONG),
        );
        painter.text(
            pos2(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            fmt_axis(value, y_step),
            FontId::proportional(9.5),
            t::MUTED,
        );
        value += y_step;
    }

    const X_TICKS: usize = 4;
    for i in 0..=X_TICKS {
        let ratio = i as f32 / X_TICKS as f32;
        let x = plot.left() + plot.width() * ratio;
        painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], grid);
        let time = t_max * ratio as f64;
        let align = match i {
            0 => Align2::LEFT_TOP,
            X_TICKS => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        let tick_text = fmt_time(time, t_max);
        let text = if i == X_TICKS {
            format!("{tick_text} s")
        } else {
            tick_text
        };
        painter.text(
            pos2(x, plot.bottom() + 5.0),
            align,
            text,
            FontId::proportional(9.5),
            t::MUTED,
        );
    }

    // 阶段分界：单曲线标出加速/匀速/减速起点，综合曲线标出各周期分界。
    for mark in marks {
        if mark.time <= 0.0 || mark.time >= t_max {
            continue;
        }
        let x = x_of(mark.time);
        dashed_vline(&painter, x, plot.top(), plot.bottom());
    }

    // 零线
    if y_min < 0.0 && y_max > 0.0 {
        let y = y_of(0.0);
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.2, t::BORDER_STRONG),
        );
    }

    // 面积填充：从零线向曲线做竖向渐变（底部近乎透明、贴近曲线处较实），
    // 比整块纯色更有体积感；正负力分别向零线两侧填充。
    let zero_y = y_of(0.0).clamp(plot.top(), plot.bottom());
    gradient_area(&painter, samples, &x_of, &y_of, zero_y, color);

    // 折线
    let points: Vec<Pos2> = samples
        .iter()
        .map(|(x, y)| pos2(x_of(*x), y_of(*y)))
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.2, color)));

    // 阶段标签放在最后绘制，避免被折线或面积填充盖住。
    for mark in marks {
        if mark.time <= 0.0 || mark.time >= t_max || mark.label.is_empty() {
            continue;
        }
        let x = x_of(mark.time);
        let galley =
            painter.layout_no_wrap(mark.label.clone(), FontId::proportional(9.0), t::TEXT_2);
        let chip = Rect::from_min_size(
            pos2(x + 3.0, plot.top() + 1.0),
            galley.size() + egui::vec2(8.0, 3.0),
        );
        painter.rect_filled(chip, egui::CornerRadius::same(4), t::SURFACE);
        painter.rect_stroke(
            chip,
            egui::CornerRadius::same(4),
            Stroke::new(1.0, t::BORDER),
            egui::StrokeKind::Inside,
        );
        painter.galley(chip.min + egui::vec2(4.0, 1.5), galley, t::TEXT_2);
    }

    painter.text(
        plot.left_top() + egui::vec2(2.0, -2.0),
        Align2::LEFT_BOTTOM,
        unit,
        FontId::proportional(10.0),
        t::MUTED,
    );
}

/// 曲线与零线之间的渐变填充；每段用两个三角形拼出梯形，顶点颜色不同形成渐变。
fn gradient_area(
    painter: &egui::Painter,
    samples: &[(f64, f64)],
    x_of: &impl Fn(f64) -> f32,
    y_of: &impl Fn(f64) -> f32,
    zero_y: f32,
    color: Color32,
) {
    let near_curve = color.gamma_multiply(0.26);
    let near_zero = color.gamma_multiply(0.02);
    let mut mesh = egui::Mesh::default();
    for pair in samples.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        let index = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x_of(x0), zero_y), near_zero);
        mesh.colored_vertex(pos2(x_of(x1), zero_y), near_zero);
        mesh.colored_vertex(pos2(x_of(x0), y_of(y0)), near_curve);
        mesh.colored_vertex(pos2(x_of(x1), y_of(y1)), near_curve);
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index + 2, index + 1, index + 3);
    }
    painter.add(Shape::mesh(mesh));
}

fn dashed_vline(painter: &egui::Painter, x: f32, top: f32, bottom: f32) {
    let mut y = top;
    while y < bottom {
        let end = (y + 4.0).min(bottom);
        painter.line_segment(
            [pos2(x, y), pos2(x, end)],
            Stroke::new(1.0, t::BORDER_STRONG),
        );
        y += 8.0;
    }
}

/// 选一个「好看」的纵轴范围与步长：步长只取 1/2/5×10^n，端点对齐到步长。
fn nice_range(min: f64, max: f64) -> (f64, f64, f64) {
    let span = (max - min).abs();
    if !span.is_finite() || span < 1e-9 {
        // 全部数值相同：给一个固定区间，避免除零。
        return (min - 0.5, max + 0.5, 1.0);
    }
    let mut step = nice_step(span / 4.0);
    loop {
        let lo = (min / step).floor() * step;
        let hi = (max / step).ceil() * step;
        if (hi - lo) / step <= 6.0 {
            return (lo, hi, step);
        }
        step *= 2.0;
    }
}

fn nice_step(raw: f64) -> f64 {
    if !raw.is_finite() || raw <= 0.0 {
        return 1.0;
    }
    let base = 10f64.powf(raw.log10().floor());
    let norm = raw / base;
    let multiplier = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 2.5 {
        2.5
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    base * multiplier
}

/// 同一根轴上的刻度使用相同小数位（由步长决定），避免 10.0 与 5.00 混排。
fn fmt_axis(x: f64, step: f64) -> String {
    let decimals = decimals_for(step);
    format!("{x:.decimals$}")
}

/// 步长本身需要几位小数才能精确表示（2.5 需要 1 位，0.05 需要 2 位）。
fn decimals_for(step: f64) -> usize {
    for decimals in 0..=4 {
        let scaled = step * 10f64.powi(decimals);
        if (scaled - scaled.round()).abs() < 1e-6 {
            return decimals as usize;
        }
    }
    4
}

/// 时间轴按最大时间统一精度，避免同一轴混用 1 位与 2 位小数。
fn fmt_time(x: f64, t_max: f64) -> String {
    if t_max >= 100.0 {
        format!("{x:.0}")
    } else if t_max >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_step_uses_1_2_5_decades() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(14.0), 20.0);
        assert_eq!(nice_step(0.03), 0.05);
        // 2.5×10ⁿ 让 98.2 的范围得到 25 的步长，而不是只有 3 条刻度的 50。
        assert_eq!(nice_step(24.545), 25.0);
    }

    #[test]
    fn axis_labels_share_precision_for_a_step() {
        assert_eq!(fmt_axis(10.0, 5.0), "10");
        assert_eq!(fmt_axis(5.0, 5.0), "5");
        assert_eq!(fmt_axis(0.8, 0.2), "0.8");
        assert_eq!(fmt_axis(0.625, 0.05), "0.62");
        assert_eq!(fmt_axis(1.5, 2.5), "1.5");
        assert_eq!(fmt_axis(0.25, 0.25), "0.25");
    }

    #[test]
    fn time_axis_uses_one_precision_per_chart() {
        assert_eq!(fmt_time(1.875, 2.5), "1.88");
        assert_eq!(fmt_time(2.5, 2.5), "2.50");
        assert_eq!(fmt_time(2.625, 10.5), "2.6");
    }

    #[test]
    fn nice_range_covers_data_and_limits_ticks() {
        let (lo, hi, step) = nice_range(-7.8, 9.7);
        assert!(lo <= -7.8);
        assert!(hi >= 9.7);
        assert!((hi - lo) / step <= 6.0);
    }

    #[test]
    fn flat_data_gets_non_zero_range() {
        let (lo, hi, _step) = nice_range(0.0, 0.0);
        assert!(hi > lo);
    }
}
