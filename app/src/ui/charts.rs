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
    painter.rect_filled(rect, egui::CornerRadius::same(8), t::SURFACE_ALT);

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
    let mut value = y_min;
    while value <= y_max + y_step * 0.5 {
        let y = y_of(value);
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, t::BORDER),
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
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, t::BORDER),
        );
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
        if !mark.label.is_empty() {
            painter.text(
                pos2(x + 3.0, plot.top() + 1.0),
                Align2::LEFT_TOP,
                &mark.label,
                FontId::proportional(9.0),
                t::MUTED,
            );
        }
    }

    // 零线
    if y_min < 0.0 && y_max > 0.0 {
        let y = y_of(0.0);
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.2, t::BORDER_STRONG),
        );
    }

    // 面积填充：用密集竖线近似，支持正负力且不受凹多边形限制。
    let zero_y = y_of(0.0).clamp(plot.top(), plot.bottom());
    let fill = color.gamma_multiply(0.10);
    let bar_width = (plot.width() / samples.len() as f32).max(1.0);
    for (x, y) in samples {
        let px = x_of(*x);
        painter.line_segment(
            [pos2(px, zero_y), pos2(px, y_of(*y))],
            Stroke::new(bar_width, fill),
        );
    }

    // 折线
    let points: Vec<Pos2> = samples
        .iter()
        .map(|(x, y)| pos2(x_of(*x), y_of(*y)))
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.0, color)));

    painter.text(
        plot.left_top() + egui::vec2(2.0, -2.0),
        Align2::LEFT_BOTTOM,
        unit,
        FontId::proportional(10.0),
        t::MUTED,
    );
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
