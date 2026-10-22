"""StayProductive GTK4 / libadwaita application."""

from __future__ import annotations

import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

from .engine import Engine, EngineError  # noqa: E402
from .window import MainWindow  # noqa: E402

APP_ID = "io.github.stayproductive.StayProductive"


class StayProductiveApp(Adw.Application):
    def __init__(self) -> None:
        super().__init__(
            application_id=APP_ID,
            flags=Gio.ApplicationFlags.DEFAULT_FLAGS,
        )
        self.engine: Engine | None = None
        self.window: MainWindow | None = None
        self.connect("activate", self._activate)
        self.connect("shutdown", self._shutdown)
        GLib.set_application_name("StayProductive")

    def _activate(self, _application: Adw.Application) -> None:
        if self.window is not None:
            self.window.present()
            return
        try:
            if self.engine is None:
                self.engine = Engine()
        except EngineError as error:
            self._show_startup_error(str(error))
            return
        self.window = MainWindow(self, self.engine)
        self.window.present()

    def _shutdown(self, _application: Adw.Application) -> None:
        if self.engine is not None:
            self.engine.close()
            self.engine = None

    def _show_startup_error(self, message: str) -> None:
        window = Adw.ApplicationWindow(application=self, title="StayProductive")
        window.set_default_size(620, 420)
        toolbar = Adw.ToolbarView()
        toolbar.add_top_bar(Adw.HeaderBar())
        page = Adw.StatusPage(
            icon_name="dialog-error-symbolic",
            title="The local state service could not start",
            description=message,
        )
        quit_button = Gtk.Button(label="Quit")
        quit_button.add_css_class("suggested-action")
        quit_button.set_halign(Gtk.Align.CENTER)
        quit_button.connect("clicked", lambda _button: self.quit())
        page.set_child(quit_button)
        toolbar.set_content(page)
        window.set_content(toolbar)
        window.present()
        self.window = window  # type: ignore[assignment]


def main() -> int:
    app = StayProductiveApp()
    return app.run(sys.argv)


if __name__ == "__main__":
    raise SystemExit(main())
