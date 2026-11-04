// A test rig for a screen's reach into the app (doc 43 cut TS.19).
//
// A screen is handed the whole `GameApp`, and the rule for some of them is a
// whitelist: the members they may touch, so that an identity store or a send
// path is never reached by accident. `onlyReaches` wraps the stand-in app a
// mount passes, so EVERY test in that file enforces the whitelist on every
// path it drives, child components included: reading any other member
// throws, failing the test that reached it. It replaces the source probes
// that regex-matched `app.<member>` in the component's text, which saw
// neither a child's reach nor one through an alias.

/** `app`, refusing every string member outside `allowed`; `reached` collects
 * the members actually read, for a test that pins the whole set. */
export function onlyReaches<T extends object>(
  app: T,
  allowed: readonly string[],
): { app: T; reached: Set<string> } {
  const reached = new Set<string>();
  const proxy = new Proxy(app, {
    get(target, key, receiver) {
      if (typeof key === "string") {
        if (!allowed.includes(key)) {
          throw new Error(`the screen reached app.${key}, outside its whitelist`);
        }
        reached.add(key);
      }
      return Reflect.get(target, key, receiver);
    },
  });
  return { app: proxy, reached };
}
