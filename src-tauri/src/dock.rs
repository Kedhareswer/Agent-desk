//! Where the overlay sits. It always hugs one edge of the screen; dragging
//! it moves it along that edge or snaps it to another one.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Pick the edge nearest to where a window was dropped. `along` is how far
/// along that edge the grab point sits, from 0.0 to 1.0. `grip` is the
/// height of the notch, which is what stays put on the side edges while the
/// answer panel grows below it.
pub fn snap(win: Rect, screen: Rect, grip: i32) -> (Edge, f64) {
    let cx = win.x + win.w / 2;
    let distances = [
        (Edge::Top, win.y - screen.y),
        (Edge::Bottom, screen.y + screen.h - (win.y + win.h)),
        (Edge::Left, win.x - screen.x),
        (Edge::Right, screen.x + screen.w - (win.x + win.w)),
    ];
    let edge = distances
        .iter()
        .min_by_key(|d| d.1)
        .map(|d| d.0)
        .unwrap_or(Edge::Top);
    let along = match edge {
        Edge::Top | Edge::Bottom => (cx - screen.x) as f64 / screen.w.max(1) as f64,
        Edge::Left | Edge::Right => (win.y + grip / 2 - screen.y) as f64 / screen.h.max(1) as f64,
    };
    (edge, along.clamp(0.0, 1.0))
}

/// Top-left corner for a window of `size` docked at `edge`, kept on screen.
pub fn place(edge: Edge, along: f64, size: (i32, i32), screen: Rect, grip: i32) -> (i32, i32) {
    let (w, h) = size;
    let max_x = (screen.x + screen.w - w).max(screen.x);
    let max_y = (screen.y + screen.h - h).max(screen.y);
    let centred_x = (screen.x + (along * screen.w as f64) as i32 - w / 2).clamp(screen.x, max_x);
    let side_y = (screen.y + (along * screen.h as f64) as i32 - grip / 2).clamp(screen.y, max_y);
    match edge {
        Edge::Top => (centred_x, screen.y),
        Edge::Bottom => (centred_x, max_y),
        Edge::Left => (screen.x, side_y),
        Edge::Right => (max_x, side_y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1040,
    };

    #[test]
    fn default_sits_top_centre() {
        assert_eq!(place(Edge::Top, 0.5, (400, 60), SCREEN, 60), (760, 0));
    }

    #[test]
    fn bottom_dock_grows_upward() {
        let small = place(Edge::Bottom, 0.5, (400, 60), SCREEN, 60);
        let tall = place(Edge::Bottom, 0.5, (400, 500), SCREEN, 60);
        assert_eq!(small.1 + 60, 1040);
        assert_eq!(tall.1 + 500, 1040);
    }

    #[test]
    fn side_dock_keeps_the_notch_still_and_stays_on_screen() {
        let small = place(Edge::Right, 0.3, (400, 60), SCREEN, 60);
        let tall = place(Edge::Right, 0.3, (400, 500), SCREEN, 60);
        assert_eq!(small, (1520, 282));
        assert_eq!(tall, small);
        // Near the bottom the panel is pushed up rather than cut off.
        assert_eq!(place(Edge::Left, 0.95, (400, 500), SCREEN, 60), (0, 540));
    }

    #[test]
    fn dropping_near_an_edge_snaps_to_it() {
        assert_eq!(
            snap(
                Rect {
                    x: 1500,
                    y: 300,
                    w: 400,
                    h: 60
                },
                SCREEN,
                60
            )
            .0,
            Edge::Right
        );
        assert_eq!(
            snap(
                Rect {
                    x: 700,
                    y: 900,
                    w: 400,
                    h: 60
                },
                SCREEN,
                60
            )
            .0,
            Edge::Bottom
        );
        let (edge, along) = snap(
            Rect {
                x: 200,
                y: 20,
                w: 400,
                h: 60,
            },
            SCREEN,
            60,
        );
        assert_eq!(edge, Edge::Top);
        assert!((along - 400.0 / 1920.0).abs() < 1e-9);
    }

    #[test]
    fn snap_then_place_round_trips_on_a_second_monitor() {
        let second = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1400,
        };
        let win = Rect {
            x: 1920,
            y: 600,
            w: 400,
            h: 60,
        };
        let (edge, along) = snap(win, second, 60);
        assert_eq!(edge, Edge::Left);
        assert_eq!(place(edge, along, (400, 60), second, 60), (1920, 600));
    }
}
