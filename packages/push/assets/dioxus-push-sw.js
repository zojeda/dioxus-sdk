// Default service worker for dioxus-sdk-push (Web Push).
//
// Host this file at the URL passed in `PushConfig.web_service_worker_url`
// (defaults to `/dioxus-push-sw.js`). It displays incoming pushes as
// notifications and forwards them to any open page so the Rust app can react
// via `use_push_notifications`.

self.addEventListener("push", (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch (_e) {
    payload = { notification: { body: event.data ? event.data.text() : "" } };
  }

  const notification = payload.notification || {};
  const title = notification.title || "";
  const options = {
    body: notification.body || "",
    data: payload,
  };

  event.waitUntil(
    (async () => {
      await self.registration.showNotification(title, options);
      const clients = await self.clients.matchAll({
        includeUncontrolled: true,
        type: "window",
      });
      for (const client of clients) {
        client.postMessage({
          kind: "push",
          notification: notification,
          data: payload.data || {},
          message_id: payload.message_id || null,
        });
      }
    })()
  );
});

self.addEventListener("notificationclick", (event) => {
  const payload = (event.notification && event.notification.data) || {};
  event.notification.close();

  event.waitUntil(
    (async () => {
      const clients = await self.clients.matchAll({
        includeUncontrolled: true,
        type: "window",
      });
      for (const client of clients) {
        client.postMessage({
          kind: "notificationclick",
          notification: payload.notification || {},
          data: payload.data || {},
          message_id: payload.message_id || null,
        });
      }
      if (clients.length === 0 && self.clients.openWindow) {
        await self.clients.openWindow("/");
      }
    })()
  );
});
