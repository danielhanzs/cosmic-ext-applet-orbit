// SPDX-License-Identifier: GPL-3.0-only

//! Sideways swipes reach the sideways lists inside the popup's scrolling.
//!
//! Once iced's scrollable has scrolled, it keeps every wheel and touchpad
//! event to itself for 1.5 s (a "transaction"), so a swipe along the hours
//! right after scrolling the page, or one that starts a little diagonally,
//! never reaches them. A button release ends the transaction: before a
//! mostly sideways scroll, this wrapper hands the page one, of a button no
//! one has, so the scroll then goes to what's under the pointer.

use cosmic::iced::{
    Event, Length, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, Widget, tree},
    },
};

pub struct Swipe<'a, Message, Theme, Renderer>(cosmic::iced::Element<'a, Message, Theme, Renderer>);

pub fn swipe<'a, Message, Theme, Renderer>(
    content: impl Into<cosmic::iced::Element<'a, Message, Theme, Renderer>>,
) -> Swipe<'a, Message, Theme, Renderer> {
    Swipe(content.into())
}

/// More sideways than up or down.
fn sideways(event: &Event) -> bool {
    match event {
        Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
            let (x, y) = match *delta {
                mouse::ScrollDelta::Lines { x, y } | mouse::ScrollDelta::Pixels { x, y } => (x, y),
            };
            x.abs() > y.abs()
        }
        _ => false,
    }
}

// Transparent: the content's own tree node, layout and drawing.
impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer> for Swipe<'_, Message, Theme, Renderer> {
    fn size(&self) -> Size<Length> {
        self.0.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.0.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.0.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.0.as_widget().draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn tag(&self) -> tree::Tag {
        self.0.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.0.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.0.as_widget().children()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.0.as_widget_mut().diff(tree);
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.0.as_widget_mut().operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if sideways(event) {
            // Not a button anything listens to; it only ends the transaction.
            let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Other(u16::MAX)));
            self.0.as_widget_mut().update(tree, &release, layout, cursor, renderer, clipboard, shell, viewport);
        }
        self.0.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.0.as_widget().mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.0.as_widget_mut().overlay(tree, layout, renderer, viewport, translation)
    }

    fn drag_destinations(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        dnd_rectangles: &mut cosmic::iced::advanced::clipboard::DndDestinationRectangles,
    ) {
        self.0.as_widget().drag_destinations(tree, layout, renderer, dnd_rectangles);
    }
}

impl<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a> From<Swipe<'a, Message, Theme, Renderer>>
    for cosmic::iced::Element<'a, Message, Theme, Renderer>
{
    fn from(swipe: Swipe<'a, Message, Theme, Renderer>) -> Self {
        Self::new(swipe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_sideways_scrolls() {
        let wheel = |x, y| Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x, y } });
        assert!(sideways(&wheel(-12.0, 3.0)));
        assert!(!sideways(&wheel(2.0, -9.0)));
        assert!(!sideways(&Event::Mouse(mouse::Event::CursorLeft)));
    }
}
