#!/usr/bin/env python3
"""用 ctypes 直接调用 IShellLink COM 创建带自定义图标的桌面 .lnk 快捷方式。"""
import ctypes
import ctypes.wintypes as wt

ole32 = ctypes.windll.ole32


class GUID(ctypes.Structure):
    _fields_ = [("d1", ctypes.c_ulong), ("d2", ctypes.c_ushort),
                ("d3", ctypes.c_ushort), ("d4", ctypes.c_ubyte * 8)]

    def __init__(self, s):
        a, b, c, d, e = s.replace("{", "").replace("}", "").split("-")
        self.d1 = int(a, 16)
        self.d2 = int(b, 16)
        self.d3 = int(c, 16)
        self.d4 = (ctypes.c_ubyte * 8)(*list(bytes.fromhex(d + e)))


CLSID_ShellLink = GUID("{00021401-0000-0000-C000-000000000046}")
IID_IShellLinkW = GUID("{000214F9-0000-0000-C000-000000000046}")
IID_IPersistFile = GUID("{0000010B-0000-0000-C000-000000000046}")

hr = ole32.CoInitialize(0)
if hr < 0:
    raise SystemExit("CoInitialize failed: %d" % hr)

pp = ctypes.c_void_p()
h = ole32.CoCreateInstance(ctypes.byref(CLSID_ShellLink), None, 1,
                           ctypes.byref(IID_IShellLinkW), ctypes.byref(pp))
if h != 0:
    raise SystemExit("CoCreateInstance failed: 0x%x" % h)
iface = pp.value


def get_vtable(iface_ptr):
    pp = ctypes.cast(iface_ptr, ctypes.POINTER(ctypes.c_void_p))
    return ctypes.cast(pp.contents, ctypes.POINTER(ctypes.c_void_p))


def vtable_call(idx, restype, argtypes, *args):
    vt = get_vtable(iface)
    f = ctypes.WINFUNCTYPE(restype, ctypes.c_void_p, *argtypes)(vt[idx])
    return f(iface, *args)


# IShellLinkW method indices
SL_SET_PATH = 20
SL_SET_WORKINGDIR = 9
SL_SET_ARGUMENTS = 11
SL_SET_SHOWCMD = 15
SL_SET_ICONLOCATION = 17

edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
html = r"D:\Desktop\SnapStudio\index.html"
icon = r"D:\Desktop\SnapStudio\SnapStudio.ico"
workdir = r"D:\Desktop\SnapStudio"
args = '--app="file:///D:/Desktop/SnapStudio/index.html"'

calls = [
    (SL_SET_PATH, [ctypes.c_wchar_p], edge),
    (SL_SET_ARGUMENTS, [ctypes.c_wchar_p], args),
    (SL_SET_WORKINGDIR, [ctypes.c_wchar_p], workdir),
    (SL_SET_SHOWCMD, [ctypes.c_int], 1),
    (SL_SET_ICONLOCATION, [ctypes.c_wchar_p, ctypes.c_int], (icon, 0)),
]
for idx, argtypes, val in calls:
    if isinstance(val, tuple):
        r = vtable_call(idx, ctypes.c_long, argtypes, *val)
    else:
        r = vtable_call(idx, ctypes.c_long, argtypes, val)
    if r != 0:
        raise SystemExit("ShellLink method %d failed: 0x%x" % (idx, r))

# QueryInterface IShellLink -> IPersistFile
ppf = ctypes.c_void_p()
r = vtable_call(0, ctypes.c_long,
                [ctypes.POINTER(GUID), ctypes.POINTER(ctypes.c_void_p)],
                ctypes.byref(IID_IPersistFile), ctypes.byref(ppf))
if r != 0:
    raise SystemExit("QueryInterface IPersistFile failed: 0x%x" % r)
pf = ppf.value

# IPersistFile::Save (index 6)
PF_SAVE = 6
vt = get_vtable(pf)
f = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int)(
    vt[PF_SAVE])
lnk = r"D:\Desktop\SnapStudio 截图工具.lnk"
rc = f(pf, lnk, 1)
if rc != 0:
    raise SystemExit("IPersistFile::Save failed: 0x%x" % rc)

print("OK created:", lnk)
print("target:", edge)
print("args:", args)
