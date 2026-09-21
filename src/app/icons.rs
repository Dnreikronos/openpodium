//! Small line icons drawn at the interface scale without a font dependency.

use iced::widget::{button, canvas};
use iced::{Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use super::shell;

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Sidebar,
    Search,
    Plus,
    Minus,
    Code,
    Spark,
    Agent,
    Terminal,
    Note,
    Files,
    Text,
    Link,
}

pub(super) fn view<'a, Message: 'a>(icon: Icon) -> Element<'a, Message> {
    canvas(icon).width(16).height(16).into()
}

pub(super) fn control<'a, Message: Clone + 'a>(icon: Icon) -> button::Button<'a, Message> {
    button(view(icon)).padding(8).style(shell::utility_button)
}

impl<Message> canvas::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let color = shell::muted_color(theme.extended_palette());
        let stroke = canvas::Stroke::default().with_color(color).with_width(1.35);
        let lines: &[&[(f32, f32)]] = match self {
            Self::Sidebar => &[&[(6.0, 2.0), (6.0, 14.0)]],
            Self::Search => &[&[(10.5, 10.5), (14.0, 14.0)]],
            Self::Plus => &[&[(8.0, 3.0), (8.0, 13.0)], &[(3.0, 8.0), (13.0, 8.0)]],
            Self::Minus => &[&[(3.0, 8.0), (13.0, 8.0)]],
            Self::Code => &[
                &[(5.0, 4.0), (1.5, 8.0), (5.0, 12.0)],
                &[(11.0, 4.0), (14.5, 8.0), (11.0, 12.0)],
                &[(9.5, 2.0), (6.5, 14.0)],
            ],
            Self::Spark => &[
                &[(8.0, 1.0), (8.0, 15.0)],
                &[(1.0, 8.0), (15.0, 8.0)],
                &[(3.0, 3.0), (13.0, 13.0)],
                &[(3.0, 13.0), (13.0, 3.0)],
            ],
            Self::Agent => &[
                &[(8.0, 1.0), (8.0, 3.0)],
                &[(5.0, 6.0), (5.0, 8.0)],
                &[(11.0, 6.0), (11.0, 8.0)],
                &[(5.0, 11.0), (11.0, 11.0)],
            ],
            Self::Terminal => &[
                &[(4.0, 5.0), (7.0, 8.0), (4.0, 11.0)],
                &[(9.0, 11.0), (12.0, 11.0)],
            ],
            Self::Note => &[
                &[(4.0, 5.0), (12.0, 5.0)],
                &[(4.0, 8.0), (10.0, 8.0)],
                &[(9.0, 14.0), (9.0, 10.0), (14.0, 10.0)],
            ],
            Self::Files => &[&[(1.5, 5.5), (1.5, 3.0), (6.0, 3.0), (8.0, 5.5)]],
            Self::Text => &[
                &[(1.5, 13.0), (5.5, 3.0), (9.5, 13.0)],
                &[(3.0, 9.0), (8.0, 9.0)],
                &[
                    (11.0, 7.0),
                    (14.0, 7.0),
                    (14.0, 13.0),
                    (11.0, 13.0),
                    (11.0, 10.0),
                    (14.0, 10.0),
                ],
            ],
            Self::Link => &[&[(5.0, 11.0), (11.0, 5.0)]],
        };
        for points in lines {
            let path = canvas::Path::new(|path| {
                for (index, &(x, y)) in points.iter().enumerate() {
                    if index == 0 {
                        path.move_to(Point::new(x, y));
                    } else {
                        path.line_to(Point::new(x, y));
                    }
                }
            });
            frame.stroke(&path, stroke);
        }
        let rectangles: &[(f32, f32, f32, f32)] = match self {
            Self::Sidebar | Self::Terminal => &[(1.5, 2.0, 13.0, 12.0)],
            Self::Agent => &[(1.5, 3.0, 13.0, 11.0)],
            Self::Note => &[(2.0, 1.5, 12.0, 13.0)],
            Self::Files => &[(1.5, 5.5, 13.0, 8.0)],
            Self::Link => &[(1.5, 7.0, 7.0, 7.0), (7.5, 2.0, 7.0, 7.0)],
            _ => &[],
        };
        for &(x, y, width, height) in rectangles {
            frame.stroke(
                &canvas::Path::rounded_rectangle(
                    Point::new(x, y),
                    Size::new(width, height),
                    2.0.into(),
                ),
                stroke,
            );
        }
        if matches!(self, Self::Search) {
            frame.stroke(&canvas::Path::circle(Point::new(6.5, 6.5), 4.5), stroke);
        }
        vec![frame.into_geometry()]
    }
}
