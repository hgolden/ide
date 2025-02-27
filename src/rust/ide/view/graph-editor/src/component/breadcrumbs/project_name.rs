//! This module provides a view for project's name which can be used to edit it.

use crate::prelude::*;

use crate::component::breadcrumbs::TEXT_SIZE;
use crate::component::breadcrumbs::GLYPH_WIDTH;
use crate::component::breadcrumbs::VERTICAL_MARGIN;
use crate::component::breadcrumbs::breadcrumb;

use enso_frp as frp;
use ensogl::application::Application;
use ensogl::application::shortcut;
use ensogl::application;
use ensogl::data::color;
use ensogl::display::object::ObjectOps;
use ensogl::display::shape::*;
use ensogl::display;
use ensogl::DEPRECATED_Animation;
use ensogl::gui::cursor;
use ensogl_text as text;
use ensogl_text::style::Size as TextSize;
use ensogl_theme as theme;
use logger::DefaultWarningLogger as Logger;



// =================
// === Constants ===
// =================

// This is a default value for the project name when it is created. The project name should
// always be initialized externally for the current project. If this value is visible in the UI,
// it was not set to the correct project name due to some bug.
const UNINITIALIZED_PROJECT_NAME: &str = "Project Name Uninitialized";
/// Default line height for project names.
pub const LINE_HEIGHT          : f32  = TEXT_SIZE * 1.5;



// ==================
// === Background ===
// ==================

mod background {
    use super::*;

    ensogl::define_shape_system! {
        () {
            let bg_color = color::Rgba::new(0.0,0.0,0.0,0.000_001);
            Plane().fill(bg_color).into()
        }
    }
}



// ===========
// === FRP ===
// ===========

ensogl::define_endpoints! {
    Input {
       /// Set the project name.
       set_name (String),
       /// Reset the project name to the one before editing.
       cancel_editing (),
       /// Enable editing the project name field and add a cursor at the mouse position.
       start_editing  (),
       /// Commit current project name.
       commit             (),
       outside_press      (),
       /// Indicates that this is the currenlty active breadcrumb.
       select             (),
       /// Indicates that this is not the currenlty active breadcrumb.
       deselect           (),
       /// Indicates the IDE is in edit mode. This means a click on some editable text should
       /// start editing it.
       ide_text_edit_mode (bool),
    }

    Output {
        pointer_style (cursor::Style),
        name          (String),
        width         (f32),
        mouse_down    (),
        edit_mode     (bool),
        selected      (bool),
        is_hovered    (bool),
    }
}



// ==================
// === Animations ===
// ==================

/// DEPRECATED_Animation handlers.
#[derive(Debug,Clone,CloneRef)]
pub struct Animations {
    color    : DEPRECATED_Animation<color::Rgba>,
    position : DEPRECATED_Animation<Vector3<f32>>
}

impl Animations {
    /// Constructor.
    pub fn new(network:&frp::Network) -> Self {
        let color    = DEPRECATED_Animation::new(&network);
        let position = DEPRECATED_Animation::new(&network);
        Self{color,position}
    }
}



// ========================
// === ProjectNameModel ===
// ========================

#[derive(Debug,Clone,CloneRef)]
#[allow(missing_docs)]
struct ProjectNameModel {
    app            : Application,
    logger         : Logger,
    display_object : display::object::Instance,
    view           : background::View,
    style          : StyleWatch,
    text_field     : text::Area,
    project_name   : Rc<RefCell<String>>,
}

impl ProjectNameModel {
    /// Constructor.
    fn new(app:&Application) -> Self {
        let app                   = app.clone_ref();
        let scene                 = app.display.scene();
        let logger                = Logger::new("ProjectName");
        let display_object        = display::object::Instance::new(&logger);
        // FIXME : StyleWatch is unsuitable here, as it was designed as an internal tool for shape system (#795)
        let style                 = StyleWatch::new(&scene.style_sheet);
        let base_color            = style.get_color(theme::graph_editor::breadcrumbs::transparent);
        let text_size:TextSize    = TEXT_SIZE.into();
        let text_field            = app.new_view::<text::Area>();
        text_field.set_default_color.emit(base_color);
        text_field.set_default_text_size(text_size);
        text_field.single_line(true);

        text_field.remove_from_scene_layer_DEPRECATED(&scene.layers.main);
        text_field.add_to_scene_layer_DEPRECATED(&scene.layers.breadcrumbs_text);
        text_field.hover();

        let view_logger = Logger::sub(&logger,"view_logger");
        let view        = background::View::new(&view_logger);

        scene.layers.breadcrumbs_background.add_exclusive(&view);

        let project_name = default();
        Self{app,logger,view,style,display_object,text_field,project_name}.init()
    }

    /// Compute the width of the ProjectName view.
    fn width(&self, content:&str) -> f32 {
        let glyphs = content.len();
        let width  = glyphs as f32 * GLYPH_WIDTH;
        width + breadcrumb::LEFT_MARGIN + breadcrumb::RIGHT_MARGIN + breadcrumb::PADDING * 2.0
    }

    fn update_alignment(&self, content:&str) {
        let width    = self.width(content);
        let x_left   = breadcrumb::LEFT_MARGIN + breadcrumb::PADDING;
        let x_center = x_left + width/2.0;

        let height   = LINE_HEIGHT;
        let y_top    = - VERTICAL_MARGIN - breadcrumb::VERTICAL_MARGIN - breadcrumb::PADDING;
        let y_center = y_top - height/2.0;

        self.text_field.set_position(Vector3(x_left,y_center+TEXT_SIZE/2.0,0.0));
        self.view.size.set(Vector2(width,height));
        self.view.set_position(Vector3(x_center,y_center,0.0));
    }

    fn init(self) -> Self {
        self.add_child(&self.text_field);
        self.add_child(&self.view);
        self.update_text_field_content(self.project_name.borrow().as_str());
        self
    }

    /// Revert the text field content to the last committed project name.
    fn reset_name(&self) {
        debug!(self.logger, "Resetting project name.");
        self.update_text_field_content(self.project_name.borrow().as_str());
    }

    /// Update the visible content of the text field.
    fn update_text_field_content(&self, content:&str) {
        self.text_field.set_content(content);
        self.update_alignment(content);
    }

    fn set_color(&self, value:color::Rgba) {
        self.text_field.set_default_color(value);
        self.text_field.set_color_all(value);
    }

    fn set_position(&self, value:Vector3<f32>) {
        self.text_field.set_position(value);
    }

    /// Change the text field content and commit the given name.
    fn rename(&self, name:impl Str) {
        let name = name.into();
        debug!(self.logger, "Renaming: '{name}'.");
        self.update_text_field_content(&name);
        self.commit(name);
    }

    /// Confirm the given name as the current project name.
    fn commit<T:Into<String>>(&self, name:T) {
        let name = name.into();
        debug!(self.logger, "Committing name: '{name}'.");
        *self.project_name.borrow_mut() = name;
    }
}

impl display::Object for ProjectNameModel {
    fn display_object(&self) -> &display::object::Instance {
        &self.display_object
    }
}



// ===================
// === ProjectName ===
// ===================

/// The view used for displaying and renaming it.
#[derive(Debug,Clone,CloneRef)]
#[allow(missing_docs)]
pub struct ProjectName {
    model   : Rc<ProjectNameModel>,
    pub frp : Frp
}

impl ProjectName {
    /// Constructor.
    fn new(app:&Application) -> Self {
        let frp     = Frp::new();
        let model   = Rc::new(ProjectNameModel::new(app));
        let network = &frp.network;
        let scene   = app.display.scene();
        let text    = &model.text_field.frp;
        // FIXME : StyleWatch is unsuitable here, as it was designed as an internal tool for shape system (#795)
        let styles           = StyleWatch::new(&scene.style_sheet);
        let hover_color      = styles.get_color(theme::graph_editor::breadcrumbs::hover);
        let deselected_color = styles.get_color(theme::graph_editor::breadcrumbs::deselected::left);
        let selected_color   = styles.get_color(theme::graph_editor::breadcrumbs::selected);
        let animations       = Animations::new(&network);

        frp::extend! { network

            // === Mouse IO ===

            frp.source.is_hovered <+ bool(&model.view.events.mouse_out,
                                          &model.view.events.mouse_over);
            frp.source.mouse_down <+ model.view.events.mouse_down;

            not_selected               <- frp.output.selected.map(|selected| !selected);
            mouse_over_if_not_selected <- model.view.events.mouse_over.gate(&not_selected);
            mouse_out_if_not_selected  <- model.view.events.mouse_out.gate(&not_selected);
            eval_ mouse_over_if_not_selected(
                animations.color.set_target_value(hover_color);
            );
            eval_ mouse_out_if_not_selected(
                animations.color.set_target_value(deselected_color);
            );
            on_deselect <- not_selected.gate(&not_selected).constant(());

            edit_click    <- model.view.events.mouse_down.gate(&frp.ide_text_edit_mode);
            start_editing <- any(edit_click,frp.input.start_editing);
            eval_ start_editing ({
                text.set_focus(true);
                text.set_cursor_at_mouse_position()
            });
            frp.source.edit_mode <+ start_editing.to_true();

            outside_press <- any(&frp.outside_press,&frp.deselect);


            // === Text Area ===

            text_content <- text.content.map(|txt| txt.to_string());
            eval text_content((content) model.update_alignment(&content));
            text_width <- text_content.map(f!((content) model.width(content)));
            frp.source.width <+ text_width;


            // === Input Commands ===

            eval_ frp.input.cancel_editing(model.reset_name());
            eval  frp.input.set_name((name) {model.rename(name)});
            frp.output.source.name <+ frp.input.set_name;


            // === Commit ===

            do_commit <- any(&frp.commit,&outside_press).gate(&frp.output.edit_mode);
            commit_text <- text_content.sample(&do_commit);
            frp.output.source.name <+ commit_text;
            eval commit_text((text) model.commit(text));
            on_commit <- commit_text.constant(());

            end_edit_mode <- any(&on_commit,&on_deselect);
            frp.output.source.edit_mode <+ end_edit_mode.to_false();


            // === Selection ===

            eval_ frp.select( animations.color.set_target_value(selected_color) );
            frp.output.source.selected <+ frp.select.to_true();

            set_inactive <- any3(&frp.deselect,&end_edit_mode,&outside_press);
            eval_ set_inactive ([text,animations]{
                text.set_focus(false);
                text.remove_all_cursors();
                animations.color.set_target_value(deselected_color);
            });
            frp.output.source.selected <+ set_inactive.to_false();


            // === Animations ===

            eval animations.color.value((value) model.set_color(*value));
            eval animations.position.value((value) model.set_position(*value));


             // === Pointer style ===

             editable <- all(&frp.output.edit_mode,&frp.ide_text_edit_mode).map(|(a,b)| *a || *b);
             on_mouse_over_and_editable <- all(frp.output.is_hovered,editable).map(|(a,b)| *a && *b);
             mouse_over_while_editing <- on_mouse_over_and_editable.gate(&on_mouse_over_and_editable);
             frp.output.source.pointer_style <+ mouse_over_while_editing.map(|_|
                cursor::Style::new_text_cursor()
             );
             no_mouse_or_edit <- on_mouse_over_and_editable.gate_not(&on_mouse_over_and_editable);
             frp.output.source.pointer_style <+ no_mouse_or_edit.map(|_|
                cursor::Style::default()
             );
             frp.output.source.pointer_style <+ frp.input.start_editing.gate(&frp.output.is_hovered).map(|_|
                cursor::Style::new_text_cursor()
             );
        }

        frp.deselect();
        frp.input.set_name.emit(UNINITIALIZED_PROJECT_NAME.to_string());

        Self{frp,model}
    }

}

impl display::Object for ProjectName {
    fn display_object(&self) -> &display::object::Instance {
        &self.model.display_object
    }
}

impl Deref for ProjectName {
    type Target = Frp;
    fn deref(&self) -> &Self::Target {
        &self.frp
    }
}

impl application::command::FrpNetworkProvider for ProjectName {
    fn network(&self) -> &frp::Network { &self.frp.network }
}

impl View for ProjectName {
    fn label()               -> &'static str { "ProjectName" }
    fn new(app:&Application) -> Self         { ProjectName::new(app) }
    fn app(&self)            -> &Application { &self.model.app }

    fn default_shortcuts() -> Vec<shortcut::Shortcut> {
        use shortcut::ActionType::*;
        (&[
            (      Press,            "",             "enter",          "commit"),
            (    Release,            "",            "escape",  "cancel_editing"),
            (DoublePress,  "is_hovered", "left-mouse-button",   "start_editing"),
        ]).iter().map(|(a,b,c,d)|Self::self_shortcut_when(*a,*c,*d,*b)).collect()
    }
}
