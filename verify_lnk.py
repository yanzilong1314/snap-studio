#!/usr/bin/env python3
"""用 IShellLinkW COM 读回 .lnk 的真实属性验证。"""
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

ole32.CoInitialize(0)

# 创建 ShellLink 实例并 QueryInterface IShellLinkW
pp = ctypes.c_void_p()
hr = ole32.CoCreateInstance(ctypes.byref(CLSID_ShellLink), None, 1,
                            ctypes.byref(IID_IShellLinkW), ctypes.byref(pp))
if hr != 0:
    raise SystemExit("CoCreateInstance failed: 0x%x" % hr)
sl = pp.value


def get_vtable(ptr):
    return ctypes.cast(
        ctypes.cast(ptr, ctypes.POINTER(ctypes.c_void_p)).contents,
        ctypes.POINTER(ctypes.c_void_p))


# IShellLinkW indices: GetPath=3, GetArguments=11, GetWorkingDirectory=9,
# GetIconLocation=17 (but these are GET, signature differs)
# IPersistFile::Load = 5
vt = get_vtable(sl)
# QueryInterface -> IPersistFile
ppf = ctypes.c_void_p()
qi = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p,
    ctypes.POINTER(GUID), ctypes.POINTER(ctypes.c_void_p))(vt[0])
r = qi(sl, ctypes.byref(IID_IPersistFile), ctypes.byref(ppf))
if r != 0:
    raise SystemExit("QI IPersistFile failed: 0x%x" % r)

# IPersistFile::Load (index 5)
pf_vt = get_vtable(ppf.value)
pload = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int)(
    pf_vt[5])
lnk = r"D:\Desktop\SnapStudio 截图工具.lnk"
r = pload(ppf.value, lnk, 0)
if r != 0:
    raise SystemExit("IPersistFile::Load failed: 0x%x" % r)

# GetPath(index 3): HRESULT GetPath(LPWSTR pszFile, int cch, FIND_DATA*, flags)
buf = ctypes.create_unicode_buffer(260)
gp = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int,
    ctypes.c_void_p, ctypes.c_uint)(vt[3])
r = gp(sl, buf, 260, None, 0)
print("TargetPath :", buf.value if r == 0 else "ERR 0x%x" % r)

# GetArguments (index 10) — 注意不是 11
abuf = ctypes.create_unicode_buffer(1024)
ga = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int)(vt[10])
r = ga(sl, abuf, 1024)
print("Arguments  :", abuf.value if r == 0 else "ERR 0x%x" % r)

# GetWorkingDirectory (index 8) — 注意不是 9
wbuf = ctypes.create_unicode_buffer(260)
gw = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int)(vt[8])
r = gw(sl, wbuf, 260)
print("WorkingDir :", wbuf.value if r == 0 else "ERR 0x%x" % r)

# GetIconLocation (index 16) — 注意不是 17
ibuf = ctypes.create_unicode_buffer(260)
iidx = ctypes.c_int(0)
gi = ctypes.WINFUNCTYPE(
    ctypes.c_long, ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_int,
    ctypes.POINTER(ctypes.c_int))(vt[16])
r = gi(sl, ibuf, 260, ctypes.byref(iidx))
print("Icon       :", ibuf.value, ",", iidx.value if r == 0 else "ERR 0x%x" % r)
