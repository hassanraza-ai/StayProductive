"""Small Xlib binding used by the X11 enforcement adapter.

Only the calls the adapter needs are exposed. Nothing here decides whether an
application is blocked; it reports window facts and performs the requested
hide/raise/focus operations. The binding is intentionally dependency-free
(``ctypes`` + ``libX11``) because the GTK front end already brings its own X11
connection and no Python Xlib package is required.
"""

from __future__ import annotations

import ctypes
import ctypes.util
from dataclasses import dataclass

Display = ctypes.c_void_p
Window = ctypes.c_ulong
Atom = ctypes.c_ulong
Bool = ctypes.c_int

CLIENT_MESSAGE = 33
SUBSTRUCTURE_NOTIFY = 1 << 19
SUBSTRUCTURE_REDIRECT = 1 << 20
REVERT_TO_POINTER_ROOT = 1
CURRENT_TIME = 0
ICONIC_STATE = 3
CW_OVERRIDE_REDIRECT = 1 << 9
IS_UNMAPPED = 0
IS_VIEWABLE = 2
ANY_PROPERTY_TYPE = 0

_NON_APPLICATION_WINDOW_TYPES = {
    "_NET_WM_WINDOW_TYPE_DESKTOP",
    "_NET_WM_WINDOW_TYPE_DOCK",
    "_NET_WM_WINDOW_TYPE_TOOLBAR",
    "_NET_WM_WINDOW_TYPE_MENU",
    "_NET_WM_WINDOW_TYPE_UTILITY",
    "_NET_WM_WINDOW_TYPE_SPLASH",
    "_NET_WM_WINDOW_TYPE_NOTIFICATION",
    "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
    "_NET_WM_WINDOW_TYPE_POPUP_MENU",
    "_NET_WM_WINDOW_TYPE_TOOLTIP",
    "_NET_WM_WINDOW_TYPE_COMBO",
    "_NET_WM_WINDOW_TYPE_DND",
}


class X11Error(RuntimeError):
    """The X server could not be reached or a request failed."""


class _XClassHint(ctypes.Structure):
    _fields_ = [("res_name", ctypes.c_void_p), ("res_class", ctypes.c_void_p)]


class _XSetWindowAttributes(ctypes.Structure):
    _fields_ = [
        ("background_pixmap", ctypes.c_ulong),
        ("background_pixel", ctypes.c_ulong),
        ("border_pixmap", ctypes.c_ulong),
        ("border_pixel", ctypes.c_ulong),
        ("bit_gravity", ctypes.c_int),
        ("win_gravity", ctypes.c_int),
        ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong),
        ("backing_pixel", ctypes.c_ulong),
        ("save_under", ctypes.c_int),
        ("event_mask", ctypes.c_long),
        ("do_not_propagate_mask", ctypes.c_long),
        ("override_redirect", ctypes.c_int),
        ("colormap", ctypes.c_ulong),
        ("cursor", ctypes.c_ulong),
    ]


class _XWindowAttributes(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_int),
        ("y", ctypes.c_int),
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("border_width", ctypes.c_int),
        ("depth", ctypes.c_int),
        ("visual", ctypes.c_void_p),
        ("root", ctypes.c_ulong),
        ("class", ctypes.c_int),
        ("bit_gravity", ctypes.c_int),
        ("win_gravity", ctypes.c_int),
        ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong),
        ("backing_pixel", ctypes.c_ulong),
        ("save_under", ctypes.c_int),
        ("colormap", ctypes.c_ulong),
        ("map_installed", ctypes.c_int),
        ("map_state", ctypes.c_int),
        ("all_event_masks", ctypes.c_long),
        ("your_event_mask", ctypes.c_long),
        ("do_not_propagate_mask", ctypes.c_long),
        ("override_redirect", ctypes.c_int),
        ("screen", ctypes.c_void_p),
    ]


class _XClientMessageData(ctypes.Union):
    _fields_ = [
        ("b", ctypes.c_char * 20),
        ("s", ctypes.c_short * 10),
        ("l", ctypes.c_long * 5),
    ]


class _XClientMessageEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("window", ctypes.c_ulong),
        ("message_type", ctypes.c_ulong),
        ("format", ctypes.c_int),
        ("data", _XClientMessageData),
    ]


class _XEvent(ctypes.Union):
    _fields_ = [
        ("type", ctypes.c_int),
        ("xclient", _XClientMessageEvent),
        ("pad", ctypes.c_long * 24),
    ]


_ERROR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, Display, ctypes.c_void_p)


@dataclass(frozen=True)
class WindowGeometry:
    x: int
    y: int
    width: int
    height: int


def _configure(lib) -> None:
    lib.XOpenDisplay.argtypes = [ctypes.c_char_p]
    lib.XOpenDisplay.restype = Display
    lib.XCloseDisplay.argtypes = [Display]
    lib.XCloseDisplay.restype = ctypes.c_int
    lib.XDefaultRootWindow.argtypes = [Display]
    lib.XDefaultRootWindow.restype = Window
    lib.XInternAtom.argtypes = [Display, ctypes.c_char_p, Bool]
    lib.XInternAtom.restype = Atom
    lib.XGetAtomName.argtypes = [Display, Atom]
    lib.XGetAtomName.restype = ctypes.c_void_p
    lib.XFree.argtypes = [ctypes.c_void_p]
    lib.XFree.restype = ctypes.c_int
    lib.XFlush.argtypes = [Display]
    lib.XFlush.restype = ctypes.c_int
    lib.XSync.argtypes = [Display, Bool]
    lib.XSync.restype = ctypes.c_int
    lib.XGetWindowProperty.argtypes = [
        Display,
        Window,
        Atom,
        ctypes.c_long,
        ctypes.c_long,
        Bool,
        Atom,
        ctypes.POINTER(Atom),
        ctypes.POINTER(ctypes.c_int),
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte)),
    ]
    lib.XGetWindowProperty.restype = ctypes.c_int
    lib.XGetClassHint.argtypes = [Display, Window, ctypes.POINTER(_XClassHint)]
    lib.XGetClassHint.restype = Bool
    lib.XGetInputFocus.argtypes = [
        Display,
        ctypes.POINTER(Window),
        ctypes.POINTER(ctypes.c_int),
    ]
    lib.XGetInputFocus.restype = ctypes.c_int
    lib.XSetInputFocus.argtypes = [Display, Window, ctypes.c_int, ctypes.c_ulong]
    lib.XSetInputFocus.restype = ctypes.c_int
    lib.XGetWindowAttributes.argtypes = [
        Display,
        Window,
        ctypes.POINTER(_XWindowAttributes),
    ]
    lib.XGetWindowAttributes.restype = ctypes.c_int
    lib.XQueryTree.argtypes = [
        Display,
        Window,
        ctypes.POINTER(Window),
        ctypes.POINTER(Window),
        ctypes.POINTER(ctypes.POINTER(Window)),
        ctypes.POINTER(ctypes.c_uint),
    ]
    lib.XQueryTree.restype = ctypes.c_int
    lib.XSendEvent.argtypes = [Display, Window, Bool, ctypes.c_long, ctypes.POINTER(_XEvent)]
    lib.XSendEvent.restype = ctypes.c_int
    lib.XUnmapWindow.argtypes = [Display, Window]
    lib.XUnmapWindow.restype = ctypes.c_int
    lib.XMapWindow.argtypes = [Display, Window]
    lib.XMapWindow.restype = ctypes.c_int
    lib.XRaiseWindow.argtypes = [Display, Window]
    lib.XRaiseWindow.restype = ctypes.c_int
    lib.XMoveResizeWindow.argtypes = [Display, Window, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint]
    lib.XMoveResizeWindow.restype = ctypes.c_int
    lib.XChangeWindowAttributes.argtypes = [
        Display,
        Window,
        ctypes.c_ulong,
        ctypes.POINTER(_XSetWindowAttributes),
    ]
    lib.XChangeWindowAttributes.restype = ctypes.c_int
    lib.XCreateSimpleWindow.argtypes = [
        Display,
        Window,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_uint,
        ctypes.c_uint,
        ctypes.c_uint,
        ctypes.c_ulong,
        ctypes.c_ulong,
    ]
    lib.XCreateSimpleWindow.restype = Window
    lib.XDestroyWindow.argtypes = [Display, Window]
    lib.XDestroyWindow.restype = ctypes.c_int
    lib.XSetClassHint.argtypes = [Display, Window, ctypes.POINTER(_XClassHint)]
    lib.XSetClassHint.restype = ctypes.c_int
    lib.XStoreName.argtypes = [Display, Window, ctypes.c_char_p]
    lib.XStoreName.restype = ctypes.c_int
    lib.XSetErrorHandler.argtypes = [_ERROR_HANDLER]
    lib.XSetErrorHandler.restype = ctypes.c_void_p


class X11:
    """A separate Xlib connection used by the adapter process."""

    def __init__(self, display_name: str | None = None) -> None:
        library = ctypes.util.find_library("X11")
        if not library:
            raise X11Error("libX11 is not available.")
        self._lib = ctypes.CDLL(library)
        _configure(self._lib)
        self._errors = 0
        self._handler = _ERROR_HANDLER(self._on_error)
        self._lib.XSetErrorHandler(self._handler)
        encoded = display_name.encode("utf-8") if display_name else None
        self._display = self._lib.XOpenDisplay(encoded)
        if not self._display:
            raise X11Error(
                f"Could not open X display {display_name or '(from DISPLAY)'}."
            )
        self._root = self._lib.XDefaultRootWindow(self._display)
        self._atoms: dict[str, int] = {}

    def _on_error(self, _display, _event) -> int:
        self._errors += 1
        return 0

    # ------------------------------------------------------------------ atoms
    def atom(self, name: str) -> int:
        atom = self._atoms.get(name)
        if atom is None:
            atom = self._lib.XInternAtom(
                self._display, name.encode("utf-8"), False
            )
            self._atoms[name] = atom
        return atom

    def atom_name(self, atom: int) -> str | None:
        if not atom:
            return None
        pointer = self._lib.XGetAtomName(self._display, atom)
        if not pointer:
            return None
        try:
            return ctypes.cast(pointer, ctypes.c_char_p).value.decode(
                "utf-8", "replace"
            )
        finally:
            self._lib.XFree(pointer)

    @property
    def root(self) -> int:
        return self._root

    # -------------------------------------------------------------- properties
    def property_raw(
        self, window: int, name: str, expected_type: int = ANY_PROPERTY_TYPE
    ):
        atom = self.atom(name)
        actual_type = Atom()
        actual_format = ctypes.c_int()
        nitems = ctypes.c_ulong()
        bytes_after = ctypes.c_ulong()
        data = ctypes.POINTER(ctypes.c_ubyte)()
        status = self._lib.XGetWindowProperty(
            self._display,
            window,
            atom,
            0,
            4096,
            False,
            expected_type,
            ctypes.byref(actual_type),
            ctypes.byref(actual_format),
            ctypes.byref(nitems),
            ctypes.byref(bytes_after),
            ctypes.byref(data),
        )
        if status != 0:
            return None
        try:
            if not actual_type.value or not data:
                return None
            if actual_format.value == 8:
                return bytes(data[: nitems.value])
            if actual_format.value == 32:
                values = ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))
                return [values[index] for index in range(nitems.value)]
            return None
        finally:
            if data:
                self._lib.XFree(data)

    def property_string(self, window: int, name: str) -> str | None:
        value = self.property_raw(window, name)
        if not isinstance(value, bytes):
            return None
        return value.split(b"\x00", 1)[0].decode("utf-8", "replace").strip() or None

    def property_u32(self, window: int, name: str) -> int | None:
        value = self.property_raw(window, name)
        if isinstance(value, list) and value:
            return int(value[0])
        return None

    def property_atoms(self, window: int, name: str) -> list[int]:
        value = self.property_raw(window, name)
        if isinstance(value, list):
            return [int(item) for item in value]
        return []

    # ------------------------------------------------------------------ windows
    def window_attributes(self, window: int) -> _XWindowAttributes | None:
        attributes = _XWindowAttributes()
        if not self._lib.XGetWindowAttributes(
            self._display, window, ctypes.byref(attributes)
        ):
            return None
        return attributes

    def is_viewable(self, window: int) -> bool:
        attributes = self.window_attributes(window)
        return attributes is not None and attributes.map_state == IS_VIEWABLE

    def toplevel(self, window: int) -> int:
        if not window:
            return 0
        current = window
        for _ in range(32):
            root = Window()
            parent = Window()
            children = ctypes.POINTER(Window)()
            count = ctypes.c_uint()
            if not self._lib.XQueryTree(
                self._display,
                current,
                ctypes.byref(root),
                ctypes.byref(parent),
                ctypes.byref(children),
                ctypes.byref(count),
            ):
                return current
            if children:
                self._lib.XFree(children)
            if not parent.value or parent.value == self._root:
                return current
            current = parent.value
        return current

    def window_class(self, window: int) -> tuple[str, str]:
        hint = _XClassHint()
        if not self._lib.XGetClassHint(self._display, window, ctypes.byref(hint)):
            return ("", "")
        name = ""
        klass = ""
        try:
            if hint.res_name:
                name = ctypes.cast(hint.res_name, ctypes.c_char_p).value.decode(
                    "utf-8", "replace"
                )
            if hint.res_class:
                klass = ctypes.cast(hint.res_class, ctypes.c_char_p).value.decode(
                    "utf-8", "replace"
                )
        finally:
            if hint.res_name:
                self._lib.XFree(hint.res_name)
            if hint.res_class:
                self._lib.XFree(hint.res_class)
        return (name, klass)

    def window_pid(self, window: int) -> int | None:
        pid = self.property_u32(window, "_NET_WM_PID")
        return pid if pid and pid > 0 else None

    def window_title(self, window: int) -> str:
        return (
            self.property_string(window, "_NET_WM_NAME")
            or self.property_string(window, "WM_NAME")
            or ""
        )

    def window_type_names(self, window: int) -> set[str]:
        return {
            name
            for name in (
                self.atom_name(atom)
                for atom in self.property_atoms(window, "_NET_WM_WINDOW_TYPE")
            )
            if name
        }

    def is_application_window(self, window: int) -> bool:
        if not window or not self.is_viewable(window):
            return False
        types = self.window_type_names(window)
        if types and types & _NON_APPLICATION_WINDOW_TYPES:
            return False
        return True

    def has_window_manager(self) -> bool:
        check = self.property_u32(self._root, "_NET_SUPPORTING_WM_CHECK")
        return bool(check) and self.window_attributes(check) is not None

    def active_window(self) -> int | None:
        active = self.property_u32(self._root, "_NET_ACTIVE_WINDOW")
        if active:
            window = self.toplevel(active)
            if self.window_attributes(window) is not None:
                return window
        focus = Window()
        revert = ctypes.c_int()
        self._lib.XGetInputFocus(
            self._display, ctypes.byref(focus), ctypes.byref(revert)
        )
        if focus.value in (0, 1):
            return None
        return self.toplevel(focus.value)

    # ------------------------------------------------------------------ actions
    def _client_message(
        self, window: int, message_name: str, values: tuple[int, int, int, int, int]
    ) -> None:
        event = _XEvent()
        event.xclient.type = CLIENT_MESSAGE
        event.xclient.send_event = True
        event.xclient.display = self._display
        event.xclient.window = window
        event.xclient.message_type = self.atom(message_name)
        event.xclient.format = 32
        for index, value in enumerate(values):
            event.xclient.data.l[index] = value
        self._lib.XSendEvent(
            self._display,
            self._root,
            False,
            SUBSTRUCTURE_REDIRECT | SUBSTRUCTURE_NOTIFY,
            ctypes.byref(event),
        )
        self._lib.XFlush(self._display)

    def hide(self, window: int) -> None:
        """Minimise through the window manager, or unmap when there is none."""

        if not window:
            return
        if self.has_window_manager():
            # ICCCM 4.1.4: ask the window manager to iconify the window.
            self._client_message(window, "WM_CHANGE_STATE", (ICONIC_STATE, 0, 0, 0, 0))
        else:
            self._lib.XUnmapWindow(self._display, window)
            self._lib.XFlush(self._display)

    def focus(self, window: int) -> None:
        if not window:
            return
        self._client_message(window, "_NET_ACTIVE_WINDOW", (1, CURRENT_TIME, 0, 0, 0))
        self._lib.XRaiseWindow(self._display, window)
        self._lib.XSetInputFocus(
            self._display, window, REVERT_TO_POINTER_ROOT, CURRENT_TIME
        )
        self._lib.XFlush(self._display)

    def map_window(self, window: int) -> None:
        if window:
            self._lib.XMapWindow(self._display, window)
            self._lib.XFlush(self._display)

    def raise_window(self, window: int) -> None:
        if window:
            self._lib.XRaiseWindow(self._display, window)
            self._lib.XFlush(self._display)

    def set_override_redirect(self, window: int) -> None:
        attributes = _XSetWindowAttributes()
        attributes.override_redirect = True
        self._lib.XChangeWindowAttributes(
            self._display,
            window,
            CW_OVERRIDE_REDIRECT,
            ctypes.byref(attributes),
        )
        self._lib.XFlush(self._display)

    def configure(self, window: int, geometry: WindowGeometry) -> None:
        self._lib.XMoveResizeWindow(
            self._display,
            window,
            geometry.x,
            geometry.y,
            max(1, geometry.width),
            max(1, geometry.height),
        )
        self._lib.XFlush(self._display)

    def sync(self) -> None:
        self._lib.XSync(self._display, False)

    # -------------------------------------------------------------- test helpers
    def create_simple_window(
        self, name: str, klass: str, geometry: WindowGeometry
    ) -> int:
        window = self._lib.XCreateSimpleWindow(
            self._display,
            self._root,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            0,
            0,
            0,
        )
        name_buffer = ctypes.create_string_buffer(name.encode("utf-8"))
        class_buffer = ctypes.create_string_buffer(klass.encode("utf-8"))
        hint = _XClassHint(
            res_name=ctypes.cast(name_buffer, ctypes.c_void_p),
            res_class=ctypes.cast(class_buffer, ctypes.c_void_p),
        )
        self._lib.XSetClassHint(self._display, window, ctypes.byref(hint))
        self._lib.XStoreName(self._display, window, name.encode("utf-8"))
        self._lib.XMapWindow(self._display, window)
        self._lib.XFlush(self._display)
        return window

    def destroy_window(self, window: int) -> None:
        if window:
            self._lib.XDestroyWindow(self._display, window)
            self._lib.XFlush(self._display)

    def close(self) -> None:
        if self._display:
            self._lib.XCloseDisplay(self._display)
            self._display = None
