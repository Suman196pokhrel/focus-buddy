"""Run under dbus-run-session with a test tray host and isolated settings."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from gi.repository import Gio, GLib

ROOT = Path(__file__).resolve().parents[1]
APP = "dev.focusbuddy.FocusBuddy"
APP_PATH = "/dev/focusbuddy/FocusBuddy"
HOST = "org.kde.StatusNotifierWatcher"
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
context = GLib.MainContext.default()
registered = []


def wait_for(check, message, timeout=15):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        try:
            result = check()
            if result:
                return result
        except GLib.Error:
            pass
        time.sleep(0.03)
    raise AssertionError(message)


def call(name, path, interface, method, params=None):
    return bus.call_sync(name, path, interface, method, params, None, Gio.DBusCallFlags.NONE, 2000, None).unpack()


def host_name(method):
    args = GLib.Variant("(su)", (HOST, 0)) if method == "RequestName" else GLib.Variant("(s)", (HOST,))
    return call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", method, args)


xml = '''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
</interface></node>'''


def method(connection, sender, path, interface, name, params, invocation):
    service = params.unpack()[0]
    registered.append(sender if service.startswith("/") else service)
    invocation.return_value(GLib.Variant("()", ()))


def get_property(connection, sender, path, interface, name):
    return {"IsStatusNotifierHostRegistered": GLib.Variant("b", True),
            "RegisteredStatusNotifierItems": GLib.Variant("as", registered),
            "ProtocolVersion": GLib.Variant("i", 0)}[name]


bus.register_object("/StatusNotifierWatcher", Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0], method, get_property, None)
host_name("RequestName")
data = ROOT / "target/desktop-test-data"
state_path = data / "focus-buddy/state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)
zone = {"mean": [0, 0, 0, 0], "spread": [0.05, 0.06, 0.05, 0.06]}
state_path.write_text(json.dumps({"version": 1, "notes": False, "zones": [None, None, zone, None]}))
env = os.environ | {"XDG_DATA_HOME": str(data), "GSK_RENDERER": "cairo", "GTK_A11Y": "none", "GIO_USE_VFS": "local", "GDK_DEBUG": "no-portals"}
log = (ROOT / "target/desktop-test.log").open("w")
screenshot = "--screenshot" in sys.argv
process = subprocess.Popen([str(ROOT / "target/debug/focus-buddy")] + (["--smoke"] if screenshot else []), cwd=ROOT, env=env, stdout=log, stderr=log)


def mapped():
    clients = json.loads(subprocess.check_output(["hyprctl", "-j", "clients"], text=True))
    return [c for c in clients if c.get("pid") == process.pid and c.get("mapped")]


def action(name):
    return call(APP, APP_PATH, "org.gtk.Actions", "Activate", GLib.Variant("(sava{sv})", (name, [], {})))


def property(name):
    return call(service, "/StatusNotifierItem", "org.freedesktop.DBus.Properties", "Get",
                GLib.Variant("(ss)", ("org.kde.StatusNotifierItem", name)))[0]


def menu_items():
    tree = call(service, menu_path, "com.canonical.dbusmenu", "GetLayout", GLib.Variant("(iias)", (0, -1, [])))[1]
    items = {}
    def visit(node):
        if isinstance(node, GLib.Variant):
            node = node.unpack()
        ident, props, children = node
        if "label" in props:
            items[props["label"]] = (ident, props)
        for child in children:
            visit(child)
    visit(tree)
    return items


def click(label):
    ident = menu_items()[label][0]
    call(service, menu_path, "com.canonical.dbusmenu", "Event", GLib.Variant("(isvu)", (ident, "clicked", GLib.Variant("s", ""), 0)))


try:
    service = wait_for(lambda: registered and registered[-1], "tray did not register")
    menu_path = property("Menu")
    wait_for(mapped, "window did not appear")
    if screenshot:
        wait_for(lambda: process.poll() is not None, "screenshot app did not exit")
        assert process.returncode == 0
        assert (ROOT / "target/ui-smoke.png").is_file()
        print("Native screenshot saved to target/ui-smoke.png")
    else:
        assert "Notebook off" in property("ToolTip")[2]
        click("Notebook mode")
        wait_for(lambda: json.loads(state_path.read_text())["notes"], "tray notebook toggle did not save")
        assert json.loads(state_path.read_text())["zones"][2] == zone
        wait_for(lambda: "Notebook on" in property("ToolTip")[2], "tooltip did not update")
        click("Notebook mode")
        wait_for(lambda: not json.loads(state_path.read_text())["notes"], "notebook mode did not disable")
        assert json.loads(state_path.read_text())["zones"][2] == zone
        action("hide")
        wait_for(lambda: not mapped(), "closing did not hide the window")
        assert process.poll() is None, "closing quit the app"
        click("Notebook mode")
        wait_for(lambda: json.loads(state_path.read_text())["notes"], "notebook toggle failed while hidden")
        assert not mapped(), "changing a tray mode reopened the window"
        click("Notebook mode")
        wait_for(lambda: not json.loads(state_path.read_text())["notes"], "notebook mode did not disable while hidden")
        call(service, "/StatusNotifierItem", "org.kde.StatusNotifierItem", "Activate", GLib.Variant("(ii)", (0, 0)))
        wait_for(mapped, "tray click did not reopen the window")
        action("hide")
        wait_for(lambda: not mapped(), "second close did not hide")
        call(APP, APP_PATH, "org.freedesktop.Application", "Activate", GLib.Variant("(a{sv})", ({},)))
        wait_for(lambda: len(mapped()) == 1, "launcher did not reopen exactly one window")
        action("hide")
        wait_for(lambda: not mapped(), "window did not hide before host restart")
        host_name("ReleaseName")
        wait_for(mapped, "losing the tray host did not restore the window")
        host_name("RequestName")
        wait_for(lambda: len(registered) >= 2, "tray did not reconnect to host")
        click("Quit Focus Buddy")
        wait_for(lambda: process.poll() is not None, "tray Quit did not terminate the app")
        assert process.returncode == 0, (ROOT / "target/desktop-test.log").read_text()
        print("Desktop integration passed: mode persistence, tray menu, hide/reopen, launcher reactivation, host recovery, and Quit.")
finally:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    log.close()
