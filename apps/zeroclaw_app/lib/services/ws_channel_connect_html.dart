import 'package:web_socket_channel/html.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

WebSocketChannel connectWs(Uri uri, {Map<String, dynamic>? headers}) {
  // NOTE: Web platform does not support custom headers for WebSocket.
  // If auth is required, put token/key into query params.
  final key = (headers?['X-Channel-Key'] ?? '').toString().trim();
  final nextUri = key.isEmpty
      ? uri
      : uri.replace(
          queryParameters: <String, String>{
            ...uri.queryParameters,
            'channel_key': key,
          },
        );
  return HtmlWebSocketChannel.connect(nextUri);
}
