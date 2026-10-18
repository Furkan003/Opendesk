import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

abstract interface class Bridge {
  String code();
  int start(String endpoint, String code, bool relay, bool host);
  Map<String, dynamic>? poll(int handle);
  bool command(int handle, Map<String, dynamic> command);
  bool frame(int handle, Uint8List jpeg);
  void close(int handle);
}

class NativeBridge implements Bridge {
  NativeBridge() : _library = DynamicLibrary.open('libopendesk.so');
  final DynamicLibrary _library;
  late final _start = _library
      .lookupFunction<
        Uint64 Function(Pointer<Char>, Pointer<Char>, Bool, Bool),
        int Function(Pointer<Char>, Pointer<Char>, bool, bool)
      >('opendesk_start');
  late final _code = _library
      .lookupFunction<Pointer<Char> Function(), Pointer<Char> Function()>(
        'opendesk_generate_code',
      );
  late final _poll = _library
      .lookupFunction<
        Pointer<Char> Function(Uint64),
        Pointer<Char> Function(int)
      >('opendesk_poll');
  late final _command = _library
      .lookupFunction<
        Bool Function(Uint64, Pointer<Char>),
        bool Function(int, Pointer<Char>)
      >('opendesk_command');
  late final _frame = _library
      .lookupFunction<
        Bool Function(Uint64, Pointer<Uint8>, UintPtr),
        bool Function(int, Pointer<Uint8>, int)
      >('opendesk_send_frame');
  late final _close = _library
      .lookupFunction<Void Function(Uint64), void Function(int)>(
        'opendesk_close',
      );
  late final _free = _library
      .lookupFunction<
        Void Function(Pointer<Char>),
        void Function(Pointer<Char>)
      >('opendesk_string_free');
  @override
  String code() {
    final pointer = _code();
    try {
      return pointer.cast<Utf8>().toDartString();
    } finally {
      _free(pointer);
    }
  }

  @override
  int start(String endpoint, String code, bool relay, bool host) {
    final a = endpoint.toNativeUtf8(), b = code.toNativeUtf8();
    try {
      return _start(a.cast(), b.cast(), relay, host);
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  @override
  Map<String, dynamic>? poll(int handle) {
    final p = _poll(handle);
    if (p.address == 0) return null;
    try {
      final event = jsonDecode(p.cast<Utf8>().toDartString());
      return event is String
          ? {event: null}
          : Map<String, dynamic>.from(event as Map);
    } finally {
      _free(p);
    }
  }

  @override
  bool command(int handle, Map<String, dynamic> command) {
    final p = jsonEncode(command).toNativeUtf8();
    try {
      return _command(handle, p.cast());
    } finally {
      calloc.free(p);
    }
  }

  @override
  bool frame(int handle, Uint8List jpeg) {
    if (jpeg.isEmpty || jpeg.length > 12 * 1024 * 1024) return false;
    final p = calloc<Uint8>(jpeg.length);
    try {
      p.asTypedList(jpeg.length).setAll(0, jpeg);
      return _frame(handle, p, jpeg.length);
    } finally {
      calloc.free(p);
    }
  }

  @override
  void close(int handle) => _close(handle);
}

(int, int)? jpegDimensions(Uint8List bytes) {
  if (bytes.length < 4 ||
      bytes.length > 12 * 1024 * 1024 ||
      bytes[0] != 255 ||
      bytes[1] != 216) {
    return null;
  }
  var offset = 2;
  while (offset + 3 < bytes.length) {
    if (bytes[offset++] != 255) return null;
    while (offset < bytes.length && bytes[offset] == 255) {
      offset++;
    }
    if (offset >= bytes.length) return null;
    final marker = bytes[offset++];
    if (marker == 217 || marker == 218) return null;
    if (marker == 1 || (marker >= 208 && marker <= 215)) continue;
    if (offset + 1 >= bytes.length) return null;
    final length = bytes[offset] * 256 + bytes[offset + 1];
    if (length < 2 || offset + length > bytes.length) return null;
    if ([192, 193, 194].contains(marker)) {
      if (length < 8) return null;
      final height = bytes[offset + 3] * 256 + bytes[offset + 4],
          width = bytes[offset + 5] * 256 + bytes[offset + 6];
      return width > 0 &&
              height > 0 &&
              width <= 8192 &&
              height <= 8192 &&
              width * height <= 32000000
          ? (width, height)
          : null;
    }
    offset += length;
  }
  return null;
}

bool acceptableJpeg(Uint8List bytes) => jpegDimensions(bytes) != null;
