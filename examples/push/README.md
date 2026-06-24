# Push Notifications Example

Learn how to use `dioxus-sdk-push`.

Run the client demo:

```sh
dx serve            # web
dx serve --platform desktop
```

On web you'll see the Web Push subscription token (after granting permission); on
Windows/Linux you'll see the fallback device id and any messages pushed over the
configured WebSocket. On Android/iOS/macOS you'll see the native FCM/APNs token.

Run the server demos (no provider credentials needed for the hub):

```sh
cargo run -p push-example --features server --bin hub
cargo run -p push-example --features server --bin send
```
