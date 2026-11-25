import { render } from "preact";
import { Api } from "./api";
import { App } from "./components/App";

// Test hook: <meta name="tally-poll-ms" content="200"> shortens the poll interval.
const ms = Number(document.querySelector('meta[name="tally-poll-ms"]')?.getAttribute("content"));
render(<App api={new Api()} pollMs={ms > 0 ? ms : 15000} />, document.getElementById("app")!);
