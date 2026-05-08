import { useEffect, useState } from "react";
import { ping } from "./api";

export function App() {
  const [msg, setMsg] = useState<string>("…");
  useEffect(() => {
    ping().then(setMsg).catch((e) => setMsg(`error: ${String(e)}`));
  }, []);
  return (
    <main style={{ fontFamily: "system-ui", padding: 16 }}>
      <h1>jfmt-viewer (M8.1 scaffold)</h1>
      <p>backend says: {msg}</p>
    </main>
  );
}
