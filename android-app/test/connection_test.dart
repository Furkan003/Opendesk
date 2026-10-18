import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:opendesk_mobile/bridge.dart';
import 'package:opendesk_mobile/main.dart';

class TestBridge implements Bridge {
  @override
  String code() => 'ABCDE-FGHJK-LMNPQ-RSTUV';
  @override
  int start(String endpoint, String code, bool relay, bool host) => 0;
  @override
  Map<String, dynamic>? poll(int handle) => null;
  @override
  bool command(int handle, Map<String, dynamic> command) => true;
  @override
  bool frame(int handle, Uint8List jpeg) => true;
  @override
  void close(int handle) {}
}

void main() {
  test('rejects malformed and oversized JPEG headers', () {
    expect(acceptableJpeg(Uint8List.fromList([1, 2, 3])), isFalse);
    final valid = Uint8List.fromList([
      255,
      216,
      255,
      192,
      0,
      8,
      8,
      0,
      32,
      0,
      32,
      1,
    ]);
    expect(jpegDimensions(valid), (32, 32));
    valid[7] = 255;
    valid[8] = 255;
    expect(acceptableJpeg(valid), isFalse);
  });
  testWidgets('shows connection setup without starting a session', (
    tester,
  ) async {
    await tester.pumpWidget(OpendeskApp(bridge: TestBridge()));
    expect(find.text('Opendesk'), findsOneWidget);
    expect(find.text('Bağlan veya paylaş'), findsOneWidget);
    expect(find.text('Bağlan'), findsOneWidget);
    expect(find.text('Paylaşımı başlat'), findsOneWidget);
    await tester.tap(find.text('TR · EN'));
    await tester.pump();
    expect(find.text('Connect or share'), findsOneWidget);
    expect(find.text('Start sharing'), findsOneWidget);
    await tester.tap(find.byTooltip('Change theme'));
    await tester.pumpAndSettle();
    expect(
      Theme.of(tester.element(find.text('Connect or share'))).brightness,
      Brightness.dark,
    );
    await tester.pumpWidget(const SizedBox());
  });
}
