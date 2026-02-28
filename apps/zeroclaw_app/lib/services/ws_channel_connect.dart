import 'package:web_socket_channel/web_socket_channel.dart';

// Conditional import wrapper.
import 'ws_channel_connect_stub.dart'
    if (dart.library.io) 'ws_channel_connect_io.dart'
    if (dart.library.html) 'ws_channel_connect_html.dart' as impl;

/// Connects a WebSocketChannel.
///
/// - On IO platforms, [headers] are supported.
/// - On Web (dart:html), headers are not supported; callers should put auth in
///   query params if needed.
WebSocketChannel connectWs(Uri uri, {Map<String, dynamic>? headers}) {
  return impl.connectWs(uri, headers: headers);
}
