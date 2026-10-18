import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'bridge.dart';

int startNative(List<dynamic> args) => NativeBridge().start(
  args[0] as String,
  args[1] as String,
  args[2] as bool,
  args[3] as bool,
);
void main() => runApp(const OpendeskApp());

class OpendeskApp extends StatefulWidget {
  const OpendeskApp({super.key, this.bridge});
  final Bridge? bridge;
  @override
  State<OpendeskApp> createState() => _OpendeskAppState();
}

class _OpendeskAppState extends State<OpendeskApp> {
  bool dark = false, turkish = true;
  ThemeData theme(bool dark) {
    final colors =
        ColorScheme.fromSeed(
          seedColor: const Color(0xff3565c4),
          brightness: dark ? Brightness.dark : Brightness.light,
        ).copyWith(
          primary: dark ? const Color(0xff8eafff) : const Color(0xff3565c4),
          surface: dark ? const Color(0xff20252e) : const Color(0xfffdfdff),
          onSurface: dark ? const Color(0xffe9edf5) : const Color(0xff252c39),
          outlineVariant: dark
              ? const Color(0xff3a4351)
              : const Color(0xffdfe4ec),
        );
    final rounded = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(10),
    );
    return ThemeData(
      useMaterial3: true,
      colorScheme: colors,
      fontFamily: 'IBM Plex Sans',
      scaffoldBackgroundColor: dark
          ? const Color(0xff171c24)
          : const Color(0xfff4f6fa),
      appBarTheme: AppBarTheme(
        backgroundColor: dark
            ? const Color(0xff171c24)
            : const Color(0xfff4f6fa),
        surfaceTintColor: Colors.transparent,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: dark ? const Color(0xff181e28) : const Color(0xfff4f6fa),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.outlineVariant),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.outlineVariant),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 14,
          vertical: 14,
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          shape: rounded,
          minimumSize: const Size(0, 44),
          textStyle: const TextStyle(
            fontFamily: 'IBM Plex Sans',
            fontWeight: FontWeight.w600,
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          shape: rounded,
          minimumSize: const Size(0, 42),
          side: BorderSide(color: colors.outlineVariant),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Opendesk',
    debugShowCheckedModeBanner: false,
    theme: theme(false),
    darkTheme: theme(true),
    themeMode: dark ? ThemeMode.dark : ThemeMode.light,
    home: Desk(
      bridge: widget.bridge,
      dark: dark,
      turkish: turkish,
      onTheme: () => setState(() => dark = !dark),
      onLanguage: () => setState(() => turkish = !turkish),
    ),
  );
}

class Desk extends StatefulWidget {
  const Desk({
    super.key,
    this.bridge,
    this.dark = false,
    this.turkish = true,
    this.onTheme,
    this.onLanguage,
  });
  final bool dark, turkish;
  final VoidCallback? onTheme, onLanguage;
  final Bridge? bridge;
  @override
  State<Desk> createState() => _DeskState();
}

class _DeskState extends State<Desk> {
  static const capture = MethodChannel('dev.opendesk/capture');
  late final Bridge bridge;
  final listenEndpoint = TextEditingController(text: '0.0.0.0:7878');
  String shareCode = '';
  bool showKeyboard = false;
  final endpoint = TextEditingController(),
      code = TextEditingController(),
      text = TextEditingController(),
      password = TextEditingController(),
      totp = TextEditingController();
  Timer? timer;
  int handle = 0, generation = 0;
  bool relay = false,
      // Code-only Internet sessions over iroh: no address and no server to run.
      internet = true,
      hosting = false,
      connected = false,
      control = false,
      busy = false,
      sharing = false,
      showSettings = true,
      unattended = false;
  String status = 'Ready', incoming = '', clipboard = '';
  Uint8List? jpeg;
  double aspect = 16 / 9;
  List<dynamic> monitors = [];
  int? monitor;
  int decodeWidth = 1920, decodeHeight = 1080;
  @override
  void initState() {
    super.initState();
    bridge = widget.bridge ?? NativeBridge();
    shareCode = bridge.code();
    capture.setMethodCallHandler(nativeCapture);
    timer = Timer.periodic(const Duration(milliseconds: 33), (_) => poll());
  }

  Future<void> nativeCapture(MethodCall call) async {
    if (call.method == 'frame' && sharing && connected && handle != 0) {
      final bytes = call.arguments as Uint8List;
      if (acceptableJpeg(bytes)) {
        bridge.frame(handle, bytes);
      }
    }
    if (call.method == 'stopped' && sharing) {
      stop();
      if (mounted) {
        setState(
          () => status = call.arguments is String
              ? call.arguments as String
              : 'Android screen sharing stopped',
        );
      }
    }
  }

  bool send(Map<String, dynamic> command) {
    if (handle == 0) return false;
    final accepted = bridge.command(handle, command);
    if (!accepted && mounted) {
      setState(() => status = 'Connection queue busy');
    }
    return accepted;
  }

  Future<void> start(bool host) async {
    if (handle != 0 || busy) return;
    final stamp = ++generation;
    setState(() {
      busy = true;
      hosting = host;
      status = 'Connecting…';
    });
    // An empty endpoint without relay selects the code-only Internet route.
    final args = <dynamic>[
      internet
          ? ''
          : host && !relay
          ? listenEndpoint.text.trim()
          : endpoint.text.trim(),
      host ? shareCode : code.text.trim(),
      relay && !internet,
      host,
    ];
    final int id;
    try {
      id = widget.bridge == null
          ? await compute(startNative, args)
          : bridge.start(
              args[0] as String,
              args[1] as String,
              args[2] as bool,
              host,
            );
    } catch (error) {
      if (mounted && stamp == generation) {
        setState(() {
          busy = false;
          status = 'Could not start connection: $error';
        });
      }
      return;
    }
    if (!mounted || stamp != generation) {
      if (id != 0) bridge.close(id);
      return;
    }
    setState(() {
      busy = false;
      handle = id;
      status = id == 0
          ? 'Could not connect. Check address and pairing code.'
          : host
          ? 'Listening; accept each incoming request'
          : 'Waiting for host acceptance';
    });
    if (id != 0 && !host && unattended) {
      send({
        'kind': 'authenticate',
        'password': password.text,
        'totp': totp.text,
      });
      password.clear();
      totp.clear();
    }
  }

  void stop() {
    generation++;
    if (handle != 0) bridge.close(handle);
    handle = 0;
    connected = false;
    control = false;
    busy = false;
    sharing = false;
    incoming = '';
    jpeg = null;
    showSettings = true;
    monitors = [];
    monitor = null;
    capture.invokeMethod<void>('stop').catchError((_) {});
    if (mounted) setState(() => status = 'Disconnected');
  }

  void poll() {
    if (handle == 0) return;
    for (var n = 0; n < 8; n++) {
      final event = bridge.poll(handle);
      if (event == null) break;
      for (final entry in event.entries) {
        final v = entry.value;
        switch (entry.key) {
          case 'Status':
            status = v as String;
          case 'Incoming':
            incoming = (v as Map)['peer'] as String;
            status = 'Request from $incoming';
          case 'Connected':
            connected = true;
            incoming = '';
            control = (v as Map)['control'] as bool;
            showSettings = false;
            status = 'Connected';
          case 'Permissions':
            control = (v as Map)['input'] == true;
          case 'Frame':
            final bytes = base64Decode((v as Map)['jpeg64'] as String);
            final size = jpegDimensions(bytes);
            if (size != null) {
              jpeg = bytes;
              aspect = size.$1 / size.$2;
              final scale = math.min(
                1.0,
                math.min(1920 / size.$1, 1080 / size.$2),
              );
              decodeWidth = math.max(1, (size.$1 * scale).round());
              decodeHeight = math.max(1, (size.$2 * scale).round());
            }
          case 'Monitors':
            monitors = v as List;
            monitor = monitors.isEmpty
                ? null
                : (monitors.first as Map)['id'] as int;
          case 'TransferOffer':
            send({'kind': 'reject-transfer'});
            status = 'File reception is not enabled in this Android preview';
          case 'Clipboard':
            clipboard = v as String;
            status = 'Received clipboard; tap Apply to accept';
          case 'Chat':
            status = 'Peer: $v';
          case 'Disconnected':
            final previous = status;
            stop();
            status = 'Session closed. $previous';
          default:
            break;
        }
      }
    }
    if (mounted) setState(() {});
  }

  Future<void> accept() async {
    if (handle == 0 || incoming.isEmpty || !hosting) return;
    final stamp = generation, acceptedHandle = handle;
    try {
      await capture.invokeMethod<void>('start');
      if (!mounted || generation != stamp || handle != acceptedHandle) {
        await capture.invokeMethod<void>('stop');
        return;
      }
      if (!send({
        'kind': 'accept',
        'permissions': {
          'screen': true,
          'input': false,
          'clipboard': false,
          'files': false,
          'audio': false,
        },
      })) {
        await capture.invokeMethod<void>('stop');
        return;
      }
      setState(() {
        sharing = true;
        incoming = '';
      });
    } on PlatformException catch (e) {
      if (mounted && generation == stamp) {
        setState(() => status = 'Screen sharing was not started: ${e.message}');
      }
    }
  }

  void pointer(Offset local, Size size) {
    if (!control || hosting || !connected) return;
    send({
      'kind': 'input',
      'event': {
        'Move': {
          'x': (local.dx / size.width).clamp(0.0, 1.0),
          'y': (local.dy / size.height).clamp(0.0, 1.0),
        },
      },
    });
  }

  void button(bool pressed) {
    if (control && !hosting && connected) {
      send({
        'kind': 'input',
        'event': {
          'Button': {'button': 0, 'pressed': pressed},
        },
      });
    }
  }

  String t(String tr, String en) => widget.turkish ? tr : en;
  String get displayStatus => switch (status) {
    'Ready' => t('Hazır', 'Ready'),
    'Connecting…' => t('Bağlanılıyor…', 'Connecting…'),
    'Connected' => t('Bağlandı', 'Connected'),
    'Disconnected' => t('Bağlantı kesildi', 'Disconnected'),
    'Listening; accept each incoming request' => t(
      'Bağlantı bekleniyor; gelen isteği onayla',
      'Listening; accept each incoming request',
    ),
    'Waiting for host acceptance' => t(
      'Karşı tarafın onayı bekleniyor',
      'Waiting for host acceptance',
    ),
    _ => status,
  };
  Color get muted => Theme.of(context).colorScheme.onSurfaceVariant;
  TextStyle get mono =>
      const TextStyle(fontFamily: 'IBM Plex Mono', fontSize: 13);
  Widget card({
    required IconData icon,
    required String title,
    required String subtitle,
    required List<Widget> children,
  }) {
    final colors = Theme.of(context).colorScheme;
    return Material(
      color: colors.surface,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(color: colors.outlineVariant),
      ),
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Container(
                  width: 40,
                  height: 40,
                  decoration: BoxDecoration(
                    color: colors.primary.withValues(alpha: .1),
                    borderRadius: BorderRadius.circular(11),
                  ),
                  child: Icon(icon, color: colors.primary, size: 21),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        style: const TextStyle(
                          fontWeight: FontWeight.w600,
                          fontSize: 17,
                        ),
                      ),
                      const SizedBox(height: 3),
                      Text(
                        subtitle,
                        style: TextStyle(fontSize: 12, color: muted),
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 20),
            ...children,
          ],
        ),
      ),
    );
  }

  Widget shareCard() => card(
    icon: Icons.screen_share_outlined,
    title: t('Bu Android ekranını paylaş', 'Share this Android screen'),
    subtitle: sharing
        ? t('Ekranın paylaşılıyor', 'Your screen is shared')
        : hosting && handle != 0
        ? t('Bağlantı bekleniyor…', 'Waiting for a connection…')
        : t('Paylaşım kapalı', 'Screen sharing is off'),
    children: [
      Text(
        t(
          'Bağlantıyı kabul ettikten sonra Android hangi ekranın paylaşılacağını sorar. Paylaşım boyunca bildirimdeki Durdur düğmesi görünür.',
          'After you accept a connection, Android asks which screen to share. A visible notification lets you stop at any time.',
        ),
        style: TextStyle(fontSize: 13, height: 1.5, color: muted),
      ),
      const SizedBox(height: 18),
      Text(
        t('DAVET KODU', 'INVITATION CODE'),
        style: TextStyle(
          fontSize: 10,
          letterSpacing: 1.1,
          color: muted,
          fontWeight: FontWeight.w600,
        ),
      ),
      const SizedBox(height: 8),
      Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: Theme.of(context).inputDecorationTheme.fillColor,
          borderRadius: BorderRadius.circular(10),
          border: Border.all(
            color: Theme.of(context).colorScheme.outlineVariant,
          ),
        ),
        child: SelectableText(
          shareCode,
          textAlign: TextAlign.center,
          style: mono.copyWith(fontSize: 16, letterSpacing: .4),
        ),
      ),
      const SizedBox(height: 8),
      Row(
        children: [
          Expanded(
            child: OutlinedButton.icon(
              onPressed: () =>
                  Clipboard.setData(ClipboardData(text: shareCode)),
              icon: const Icon(Icons.copy, size: 16),
              label: Text(t('Kodu kopyala', 'Copy code')),
            ),
          ),
          const SizedBox(width: 8),
          IconButton.filledTonal(
            tooltip: t('Yeni kod oluştur', 'Generate new code'),
            onPressed: handle == 0 && !busy
                ? () => setState(() => shareCode = bridge.code())
                : null,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      const SizedBox(height: 16),
      if (!relay && !internet)
        TextField(
          controller: listenEndpoint,
          enabled: handle == 0 && !busy,
          style: mono,
          decoration: InputDecoration(
            labelText: t(
              'Dinleme adresi · IP:port',
              'Listen address · IP:port',
            ),
          ),
        ),
      const SizedBox(height: 14),
      Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(Icons.touch_app_outlined, size: 17, color: muted),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              t(
                'Android ekran paylaşımı görüntülemeyle sınırlıdır. Bu sürümde Android kontrolü, pano, ses ve dosya aktarımı kapalıdır.',
                'Android hosting is view-only. Android control, clipboard, audio and file transfer are disabled in this build.',
              ),
              style: TextStyle(fontSize: 12, height: 1.4, color: muted),
            ),
          ),
        ],
      ),
      const SizedBox(height: 18),
      FilledButton.icon(
        onPressed: handle == 0 && !busy
            ? () => start(true)
            : hosting
            ? stop
            : null,
        icon: Icon(
          hosting && handle != 0
              ? Icons.stop_circle_outlined
              : Icons.screen_share_outlined,
          size: 18,
        ),
        label: Text(
          hosting && handle != 0
              ? t('Paylaşımı durdur', 'Stop sharing')
              : t('Paylaşımı başlat', 'Start sharing'),
        ),
      ),
    ],
  );
  Widget connectCard() => card(
    icon: Icons.cast_connected_outlined,
    title: t('Uzak bilgisayara bağlan', 'Connect to a computer'),
    subtitle: t(
      'Windows ve Linux cihazlarına güvenli bağlantı',
      'A secure connection to Windows and Linux',
    ),
    children: [
      if (!internet) ...[
        TextField(
          controller: endpoint,
          enabled: handle == 0 && !busy,
          style: mono,
          decoration: InputDecoration(
            labelText: relay
                ? t('Relay sunucusu · host:port', 'Relay server · host:port')
                : t('Uzak adres · IP:port', 'Remote address · IP:port'),
            prefixIcon: const Icon(Icons.dns_outlined, size: 18),
          ),
        ),
        const SizedBox(height: 12),
      ],
      TextField(
        controller: code,
        enabled: handle == 0 && !busy,
        style: mono,
        autocorrect: false,
        decoration: InputDecoration(
          labelText: t('Karşı cihazın davet kodu', 'Remote invitation code'),
          prefixIcon: const Icon(Icons.key_outlined, size: 18),
        ),
      ),
      const SizedBox(height: 8),
      Text(
        t(
          'Kodu cihaz sahibinden al. Her bağlantıyı karşı taraf onaylar; gözetimsiz erişim ayrıca yapılandırılır.',
          'Get the code from the device owner. Connections require host approval; unattended access must be configured separately.',
        ),
        style: TextStyle(fontSize: 12, height: 1.45, color: muted),
      ),
      const SizedBox(height: 16),
      FilledButton.icon(
        onPressed: handle == 0 && !busy ? () => start(false) : null,
        icon: const Icon(Icons.arrow_forward, size: 18),
        label: Text(t('Bağlan', 'Connect')),
      ),
      const SizedBox(height: 12),
      const Divider(height: 1),
      Theme(
        data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
        child: ExpansionTile(
          tilePadding: EdgeInsets.zero,
          childrenPadding: EdgeInsets.zero,
          title: Text(
            t('Bağlantı ayarları', 'Connection settings'),
            style: const TextStyle(fontSize: 13),
          ),
          children: [
            SwitchListTile.adaptive(
              contentPadding: EdgeInsets.zero,
              title: Text(
                t('İnternet üzerinden, yalnızca kod', 'Internet, code only'),
                style: const TextStyle(fontSize: 13),
              ),
              subtitle: Text(
                t(
                  'IP veya sunucu gerekmez; mümkünse doğrudan, değilse ücretsiz relay',
                  'No IP or server needed; direct when possible, else a free relay',
                ),
                style: const TextStyle(fontSize: 11),
              ),
              value: internet,
              onChanged: handle == 0 && !busy
                  ? (value) => setState(() => internet = value)
                  : null,
            ),
            SwitchListTile.adaptive(
              contentPadding: EdgeInsets.zero,
              title: Text(
                t('Kendi relay sunucumu kullan', 'Use my relay server'),
                style: const TextStyle(fontSize: 13),
              ),
              subtitle: Text(
                t(
                  'Her iki cihazda aynı sunucu ve kod',
                  'Use the same server and code on both devices',
                ),
                style: const TextStyle(fontSize: 11),
              ),
              value: relay && !internet,
              onChanged: handle == 0 && !busy && !internet
                  ? (value) => setState(() => relay = value)
                  : null,
            ),
            SwitchListTile.adaptive(
              contentPadding: EdgeInsets.zero,
              title: Text(
                t('Gözetimsiz erişim', 'Unattended access'),
                style: const TextStyle(fontSize: 13),
              ),
              subtitle: Text(
                t(
                  'Parola ve doğrulayıcı kodu',
                  'Password and authenticator code',
                ),
                style: const TextStyle(fontSize: 11),
              ),
              value: unattended,
              onChanged: handle == 0 && !busy
                  ? (value) => setState(() => unattended = value)
                  : null,
            ),
            if (unattended) ...[
              TextField(
                controller: password,
                obscureText: true,
                decoration: InputDecoration(
                  labelText: t('Cihaz parolası', 'Host password'),
                ),
              ),
              const SizedBox(height: 10),
              TextField(
                controller: totp,
                keyboardType: TextInputType.number,
                maxLength: 6,
                style: mono,
                decoration: InputDecoration(
                  labelText: t('Doğrulayıcı kodu', 'Authenticator code'),
                ),
              ),
            ],
          ],
        ),
      ),
    ],
  );
  Widget consentCard() => Container(
    padding: const EdgeInsets.all(18),
    decoration: BoxDecoration(
      color: Theme.of(context).colorScheme.primary.withValues(alpha: .08),
      border: Border.all(
        color: Theme.of(context).colorScheme.primary.withValues(alpha: .35),
      ),
      borderRadius: BorderRadius.circular(14),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Icon(
              Icons.person_add_alt_1_outlined,
              color: Theme.of(context).colorScheme.primary,
            ),
            const SizedBox(width: 10),
            Text(
              t('Bağlantı isteği', 'Connection request'),
              style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
            ),
          ],
        ),
        const SizedBox(height: 10),
        SelectableText(incoming, style: mono),
        const SizedBox(height: 8),
        Text(
          t(
            'Ekranını görmek istiyor. Kabul ettikten sonra Android paylaşım izni soracak.',
            'Wants to view your screen. Android will request screen-sharing permission after you accept.',
          ),
          style: TextStyle(color: muted, fontSize: 13),
        ),
        const SizedBox(height: 14),
        Wrap(
          spacing: 10,
          runSpacing: 8,
          children: [
            FilledButton.icon(
              onPressed: accept,
              icon: const Icon(Icons.check, size: 18),
              label: Text(
                t('Kabul et ve ekranı paylaş', 'Accept and share screen'),
              ),
            ),
            OutlinedButton(
              onPressed: () => send({'kind': 'reject'}),
              child: Text(t('Reddet', 'Reject')),
            ),
          ],
        ),
      ],
    ),
  );
  Widget home() => LayoutBuilder(
    builder: (context, box) => SingleChildScrollView(
      padding: EdgeInsets.symmetric(
        horizontal: box.maxWidth < 500 ? 18 : 28,
        vertical: 20,
      ),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 1000),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                t('Bağlan veya paylaş', 'Connect or share'),
                style: const TextStyle(
                  fontSize: 28,
                  fontWeight: FontWeight.w600,
                  letterSpacing: -.6,
                ),
              ),
              const SizedBox(height: 9),
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Icon(
                    Icons.verified_user_outlined,
                    size: 17,
                    color: Color(0xff32845e),
                  ),
                  const SizedBox(width: 7),
                  Expanded(
                    child: Text(
                      t(
                        'Uçtan uca şifreli. Her bağlantıyı sen onaylarsın.',
                        'End-to-end encrypted. You approve each connection.',
                      ),
                      style: TextStyle(color: muted, fontSize: 13),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 24),
              if (incoming.isNotEmpty) ...[
                consentCard(),
                const SizedBox(height: 18),
              ],
              if (box.maxWidth >= 720)
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Expanded(child: shareCard()),
                    const SizedBox(width: 18),
                    Expanded(child: connectCard()),
                  ],
                )
              else ...[
                shareCard(),
                const SizedBox(height: 18),
                connectCard(),
              ],
              const SizedBox(height: 20),
              Row(
                children: [
                  Icon(Icons.lock_outline, size: 13, color: muted),
                  const SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      t(
                        'Eşleştirme kodlarını yalnızca güvendiğin kişilerle paylaş.',
                        'Share pairing codes only with people you trust.',
                      ),
                      style: TextStyle(fontSize: 11, color: muted),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
            ],
          ),
        ),
      ),
    ),
  );
  Widget sessionView() => Column(
    children: [
      Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.surface,
          border: Border(
            bottom: BorderSide(
              color: Theme.of(context).colorScheme.outlineVariant,
            ),
          ),
        ),
        child: Wrap(
          spacing: 8,
          runSpacing: 6,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Icon(
              hosting
                  ? Icons.screen_share_outlined
                  : Icons.desktop_windows_outlined,
              size: 18,
              color: Theme.of(context).colorScheme.primary,
            ),
            Text(
              hosting
                  ? t('Android ekran paylaşımı', 'Android screen sharing')
                  : t('Uzak oturum', 'Remote session'),
              style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
            ),
            if (!hosting)
              Chip(
                visualDensity: VisualDensity.compact,
                label: Text(
                  control
                      ? t('Kontrol açık', 'Control enabled')
                      : t('Yalnızca görüntü', 'View only'),
                  style: const TextStyle(fontSize: 11),
                ),
              ),
            if (!hosting && monitors.isNotEmpty)
              DropdownButton<int>(
                value: monitor,
                underline: const SizedBox(),
                items: monitors
                    .map(
                      (m) => DropdownMenuItem<int>(
                        value: (m as Map)['id'] as int,
                        child: Text(
                          m['name'] as String,
                          style: const TextStyle(fontSize: 12),
                        ),
                      ),
                    )
                    .toList(),
                onChanged: (id) {
                  if (id != null) {
                    setState(() => monitor = id);
                    send({'kind': 'monitor', 'id': id});
                  }
                },
              ),
            if (!hosting)
              IconButton(
                tooltip: t(
                  'Klavye, sohbet ve pano',
                  'Keyboard, chat and clipboard',
                ),
                onPressed: () => setState(() => showKeyboard = !showKeyboard),
                icon: Icon(
                  showKeyboard
                      ? Icons.keyboard_hide_outlined
                      : Icons.keyboard_outlined,
                  size: 20,
                ),
              ),
            OutlinedButton.icon(
              onPressed: stop,
              icon: const Icon(Icons.call_end_outlined, size: 16),
              label: Text(t('Oturumu bitir', 'End session')),
            ),
          ],
        ),
      ),
      Expanded(
        child: Container(
          color: const Color(0xff10151e),
          child: jpeg == null
              ? Center(
                  child: Padding(
                    padding: const EdgeInsets.all(24),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          hosting
                              ? Icons.phonelink_ring_outlined
                              : Icons.desktop_windows_outlined,
                          size: 48,
                          color: const Color(0xff8eafff),
                        ),
                        const SizedBox(height: 16),
                        Text(
                          hosting
                              ? t(
                                  'Ekranın karşı tarafa paylaşılıyor',
                                  'Your screen is being shared',
                                )
                              : t(
                                  'Uzak ekran bekleniyor…',
                                  'Waiting for the remote display…',
                                ),
                          textAlign: TextAlign.center,
                          style: const TextStyle(
                            color: Color(0xffe9edf5),
                            fontSize: 16,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        const SizedBox(height: 8),
                        Text(
                          hosting
                              ? t(
                                  'İstediğin zaman bildirimden veya buradan durdur.',
                                  'Stop here or from the notification at any time.',
                                )
                              : t(
                                  'Görüntü geldiğinde burada görünecek.',
                                  'The display appears here when frames arrive.',
                                ),
                          textAlign: TextAlign.center,
                          style: const TextStyle(
                            color: Color(0xff9ba8be),
                            fontSize: 12,
                          ),
                        ),
                      ],
                    ),
                  ),
                )
              : LayoutBuilder(
                  builder: (context, box) {
                    final w = math.min(box.maxWidth, box.maxHeight * aspect),
                        h = w / aspect;
                    return Center(
                      child: SizedBox(
                        width: w,
                        height: h,
                        child: Listener(
                          onPointerHover: (e) =>
                              pointer(e.localPosition, Size(w, h)),
                          onPointerDown: (e) {
                            pointer(e.localPosition, Size(w, h));
                            button(true);
                          },
                          onPointerMove: (e) =>
                              pointer(e.localPosition, Size(w, h)),
                          onPointerUp: (_) => button(false),
                          onPointerCancel: (_) => button(false),
                          child: Image.memory(
                            jpeg!,
                            fit: BoxFit.fill,
                            gaplessPlayback: true,
                            cacheWidth: decodeWidth,
                            cacheHeight: decodeHeight,
                            errorBuilder: (_, e, stack) => Text(
                              t(
                                'Geçersiz ekran görüntüsü',
                                'Invalid display frame',
                              ),
                              style: const TextStyle(color: Colors.white),
                            ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
        ),
      ),
      if (!hosting && showKeyboard)
        Container(
          padding: const EdgeInsets.all(12),
          color: Theme.of(context).colorScheme.surface,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: text,
                      decoration: InputDecoration(
                        hintText: t(
                          'Uzak metin veya sohbet',
                          'Remote text or chat',
                        ),
                        isDense: true,
                      ),
                    ),
                  ),
                  IconButton(
                    tooltip: t('Uzaktan yaz', 'Type remotely'),
                    onPressed: control
                        ? () {
                            send({
                              'kind': 'input',
                              'event': {'Text': text.text},
                            });
                            text.clear();
                          }
                        : null,
                    icon: const Icon(Icons.keyboard),
                  ),
                  IconButton(
                    tooltip: t('Sohbet mesajı gönder', 'Send chat'),
                    onPressed: () {
                      send({'kind': 'chat', 'text': text.text});
                      text.clear();
                    },
                    icon: const Icon(Icons.send_outlined),
                  ),
                ],
              ),
              Wrap(
                spacing: 4,
                children: [
                  for (final key in ['Enter', 'Backspace', 'Escape'])
                    TextButton(
                      onPressed: control
                          ? () {
                              send({
                                'kind': 'input',
                                'event': {
                                  'Key': {'key': key, 'pressed': true},
                                },
                              });
                              send({
                                'kind': 'input',
                                'event': {
                                  'Key': {'key': key, 'pressed': false},
                                },
                              });
                            }
                          : null,
                      child: Text(key),
                    ),
                  TextButton(
                    onPressed: () async {
                      final item = await Clipboard.getData('text/plain');
                      if (item?.text != null) {
                        send({'kind': 'clipboard', 'text': item!.text});
                      }
                    },
                    child: Text(t('Panoyu gönder', 'Send clipboard')),
                  ),
                  if (clipboard.isNotEmpty)
                    TextButton(
                      onPressed: () {
                        Clipboard.setData(ClipboardData(text: clipboard));
                        setState(() => clipboard = '');
                      },
                      child: Text(t('Panoyu uygula', 'Apply clipboard')),
                    ),
                ],
              ),
            ],
          ),
        ),
    ],
  );
  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      titleSpacing: 18,
      title: Row(
        children: [
          SizedBox(
            width: 27,
            height: 27,
            child: CustomPaint(
              painter: OpendeskMark(
                Theme.of(context).colorScheme.primary,
                Theme.of(context).scaffoldBackgroundColor,
              ),
            ),
          ),
          const SizedBox(width: 10),
          const Text(
            'Opendesk',
            style: TextStyle(
              fontSize: 21,
              fontWeight: FontWeight.w600,
              letterSpacing: -.5,
            ),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: widget.onLanguage,
          child: Text(
            widget.turkish ? 'TR · EN' : 'EN · TR',
            style: mono.copyWith(fontSize: 11),
          ),
        ),
        IconButton(
          tooltip: t('Temayı değiştir', 'Change theme'),
          onPressed: widget.onTheme,
          icon: Icon(
            widget.dark ? Icons.light_mode_outlined : Icons.dark_mode_outlined,
            size: 20,
          ),
        ),
        if (connected)
          IconButton(
            tooltip: t('Ana ekran / oturum', 'Home / session'),
            onPressed: () => setState(() => showSettings = !showSettings),
            icon: Icon(
              showSettings
                  ? Icons.desktop_windows_outlined
                  : Icons.home_outlined,
              size: 20,
            ),
          ),
        const SizedBox(width: 8),
      ],
    ),
    body: SafeArea(
      child: Column(
        children: [
          Container(
            margin: const EdgeInsets.fromLTRB(18, 0, 18, 8),
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
            decoration: BoxDecoration(
              color: connected
                  ? const Color(0xff32845e).withValues(alpha: .09)
                  : Theme.of(context).colorScheme.primary
                        .withValues(alpha: .06),
              borderRadius: BorderRadius.circular(9),
            ),
            child: Row(
              children: [
                if (busy)
                  const SizedBox(
                    width: 13,
                    height: 13,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                else
                  Icon(
                    connected ? Icons.circle : Icons.info_outline,
                    size: connected ? 9 : 14,
                    color: connected ? const Color(0xff32845e) : muted,
                  ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    sharing
                        ? '${t('PAYLAŞILIYOR', 'SHARING')} · $status'
                        : displayStatus,
                    style: TextStyle(fontSize: 12, color: muted),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                if (handle != 0 || busy)
                  IconButton(
                    tooltip: t('Bağlantıyı kes', 'Disconnect'),
                    visualDensity: VisualDensity.compact,
                    onPressed: stop,
                    icon: const Icon(Icons.close, size: 16),
                  ),
              ],
            ),
          ),
          Expanded(child: connected && !showSettings ? sessionView() : home()),
        ],
      ),
    ),
  );
  @override
  void dispose() {
    timer?.cancel();
    generation++;
    if (handle != 0) bridge.close(handle);
    capture.invokeMethod<void>('stop').catchError((_) {});
    capture.setMethodCallHandler(null);
    for (final c in [listenEndpoint, endpoint, code, text, password, totp]) {
      c.dispose();
    }
    super.dispose();
  }
}

class OpendeskMark extends CustomPainter {
  OpendeskMark(this.color, this.background);
  final Color color, background;
  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.width / 28;
    canvas.save();
    canvas.scale(scale);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        const Rect.fromLTWH(1.25, 1.25, 15.5, 15.5),
        const Radius.circular(4),
      ),
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.5,
    );
    final front = RRect.fromRectAndRadius(
      const Rect.fromLTWH(10, 10, 18, 18),
      const Radius.circular(5),
    );
    canvas.drawRRect(
      front,
      Paint()
        ..color = background
        ..style = PaintingStyle.stroke
        ..strokeWidth = 5,
    );
    canvas.drawRRect(front, Paint()..color = color);
    canvas.restore();
  }

  @override
  bool shouldRepaint(OpendeskMark old) =>
      old.color != color || old.background != background;
}
