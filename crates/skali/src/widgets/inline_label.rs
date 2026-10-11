//! A label with widgets in its text, the way Basecamp shows a mention: the
//! person's face and name, wrapping with the words around it. Each widget
//! stands on an object replacement character (`richtext::OBJECT`) in the
//! markup; a Pango shape attribute makes that character as big as the
//! widget, and once the label is laid out the widget goes where its
//! character landed.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
use skali_core::richtext::{self, Mention};

mod imp {
    use super::*;
    use std::cell::{Cell, OnceCell, RefCell};

    #[derive(Default)]
    pub struct InlineLabel {
        pub label: OnceCell<gtk::Label>,
        pub children: RefCell<Vec<gtk::Widget>>,
        /// The sizes the shape attributes were made for.
        pub sizes: RefCell<Vec<(i32, i32)>>,
        /// And the font's ascent and descent, in Pango units.
        pub metrics: Cell<(i32, i32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InlineLabel {
        const NAME: &'static str = "SkaliInlineLabel";
        type Type = super::InlineLabel;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for InlineLabel {
        fn constructed(&self) {
            self.parent_constructed();
            let label = gtk::Label::builder()
                .use_markup(true)
                .wrap(true)
                .wrap_mode(pango::WrapMode::WordChar)
                .xalign(0.0)
                .build();
            label.set_parent(&*self.obj());
            self.label.set(label).unwrap();
        }

        fn dispose(&self) {
            for child in self.children.take() {
                child.unparent();
            }
            if let Some(label) = self.label.get() {
                label.unparent();
            }
        }
    }

    impl WidgetImpl for InlineLabel {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.update_shapes();
            self.label().measure(orientation, for_size)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let label = self.label();
            label.allocate(width, height, baseline, None);
            let layout = label.layout();
            let (x0, y0) = label.layout_offsets();
            let text = label.text();
            let sizes = self.sizes.borrow();
            let places = text.match_indices(richtext::OBJECT).map(|(index, _)| index);
            for ((child, &(w, h)), index) in
                self.children.borrow().iter().zip(sizes.iter()).zip(places)
            {
                // The grapheme's x and width; y and height are its line's.
                let place = layout.index_to_pos(index as i32);
                let x = x0 + place.x() / pango::SCALE;
                let y = y0 + place.y() / pango::SCALE + (place.height() / pango::SCALE - h) / 2;
                child.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
            }
        }
    }

    impl InlineLabel {
        pub fn label(&self) -> &gtk::Label {
            self.label.get().unwrap()
        }

        /// Make each object character as big as its widget, if the widgets'
        /// sizes changed (first measure, a theme or font change).
        fn update_shapes(&self) {
            let children = self.children.borrow();
            let sizes: Vec<(i32, i32)> = children
                .iter()
                .map(|child| {
                    let (_, width, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                    let (_, height, _, _) = child.measure(gtk::Orientation::Vertical, width);
                    (width, height)
                })
                .collect();
            let label = self.label();
            let font = label.pango_context().metrics(None, None);
            let metrics = (font.ascent(), font.descent());
            if *self.sizes.borrow() == sizes && self.metrics.get() == metrics {
                return;
            }
            let attributes = pango::AttrList::new();
            let text = label.text();
            let places = text.match_indices(richtext::OBJECT).map(|(index, _)| index);
            for (&(width, _), index) in sizes.iter().zip(places) {
                // As wide as the widget but only as tall as the text, so a
                // line with a mention isn't taller than one without; the
                // widget is centred on the line and may reach past it.
                let rect = pango::Rectangle::new(
                    0,
                    -metrics.0,
                    width * pango::SCALE,
                    metrics.0 + metrics.1,
                );
                let mut shape = pango::AttrShape::new(&rect, &rect);
                shape.set_start_index(index as u32);
                shape.set_end_index((index + richtext::OBJECT.len_utf8()) as u32);
                attributes.insert(shape);
            }
            label.set_attributes(Some(&attributes));
            *self.sizes.borrow_mut() = sizes;
            self.metrics.set(metrics);
        }
    }
}

glib::wrapper! {
    pub struct InlineLabel(ObjectSubclass<imp::InlineLabel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InlineLabel {
    /// Pango markup with `widgets` on its object characters, in order.
    pub fn new(markup: &str, widgets: Vec<gtk::Widget>) -> Self {
        let this: Self = glib::Object::new();
        let imp = this.imp();
        imp.label().set_markup(markup);
        for widget in &widgets {
            widget.set_parent(&this);
        }
        *imp.children.borrow_mut() = widgets;
        this
    }

    /// Rich-text markup with each mention as Basecamp shows one. Copied,
    /// a mention is its "@name".
    pub fn with_mentions(markup: &str, mentions: &[Mention]) -> Self {
        let this = Self::new(markup, mentions.iter().map(mention).collect());
        if !mentions.is_empty() {
            let names: Vec<String> = mentions.iter().map(|m| format!("@{}", m.name)).collect();
            this.label().connect_copy_clipboard(move |label| {
                let Some((start, end)) = label.selection_bounds() else {
                    return;
                };
                let (start, end) = (start.min(end) as usize, start.max(end) as usize);
                let text = label.text();
                // Objects before the selection, to know which name is next.
                let mut seen = text
                    .chars()
                    .take(start)
                    .filter(|&c| c == richtext::OBJECT)
                    .count();
                let mut copied = String::new();
                for c in text.chars().skip(start).take(end - start) {
                    if c == richtext::OBJECT {
                        copied.push_str(names.get(seen).map(String::as_str).unwrap_or_default());
                        seen += 1;
                    } else {
                        copied.push(c);
                    }
                }
                label.clipboard().set_text(&copied);
                label.stop_signal_emission_by_name("copy-clipboard");
            });
        }
        this
    }

    /// The label inside, for its links and selection.
    pub fn label(&self) -> &gtk::Label {
        self.imp().label()
    }
}

/// A mention: a small face and the name, on a rounded tint; the full name
/// as its tooltip.
pub fn mention(mention: &Mention) -> gtk::Widget {
    let pill = gtk::Box::builder()
        .spacing(4)
        .tooltip_text(&mention.full_name)
        .css_classes(["mention"])
        .build();
    let face = adw::Avatar::builder()
        .size(18)
        .text(&mention.full_name)
        .show_initials(true)
        .build();
    crate::avatars::load_into(&face, &mention.avatar_url);
    pill.append(&face);
    pill.append(&gtk::Label::new(Some(&mention.name)));
    pill.upcast()
}
