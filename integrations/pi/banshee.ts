import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "typebox";
import net from "node:net";
import os from "node:os";
import path from "node:path";

// Line-delimited JSON-RPC 2.0 over the daemon's unix socket.
const SOCKET_PATH = path.join(os.homedir(), ".banshee", "banshee.sock");
const BANSHEE_SPEAK = "banshee.speak";
const BANSHEE_ASK_USER = "banshee.ask_user";

const DAEMON_DOWN = "Banshee is not running. Start it with: banshee start";
// Guards the connect only. Once connected, ask_user legitimately sits idle for
// minutes while the question plays and the user answers.
const CONNECT_TIMEOUT_MS = 5_000;

function callDaemon(
  method: string,
  params: unknown,
  signal?: AbortSignal,
): Promise<any> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(SOCKET_PATH);
    let buffer = "";
    const onAbort = () => socket.destroy(new Error("aborted"));
    signal?.addEventListener("abort", onAbort, { once: true });
    const cleanup = () => signal?.removeEventListener("abort", onAbort);

    socket.setTimeout(CONNECT_TIMEOUT_MS, () =>
      socket.destroy(new Error(DAEMON_DOWN)),
    );
    socket.on("connect", () => {
      socket.setTimeout(0);
      socket.write(
        JSON.stringify({ jsonrpc: "2.0", method, params, id: 1 }) + "\n",
      );
    });
    socket.on("data", (chunk) => {
      buffer += chunk.toString();
      const nl = buffer.indexOf("\n");
      if (nl === -1) return; // wait for the full line
      socket.end();
      cleanup();
      try {
        const res = JSON.parse(buffer.slice(0, nl));
        if (res.error) {
          reject(new Error(res.error.message ?? String(res.error.code)));
        } else {
          resolve(res.result);
        }
      } catch (error) {
        reject(error);
      }
    });
    socket.on("error", (error: NodeJS.ErrnoException) => {
      cleanup();
      // No socket file or nothing listening: the daemon is down, not a bug.
      const down = error.code === "ENOENT" || error.code === "ECONNREFUSED";
      reject(down ? new Error(DAEMON_DOWN) : error);
    });
    // Already settled in the normal path; this catches the daemon hanging up
    // mid-call, which would otherwise leave the tool call pending forever.
    socket.on("close", () => {
      cleanup();
      reject(new Error(DAEMON_DOWN));
    });
  });
}

// A thrown execute() lands here as content, e.g. daemon down or microphone unavailable.
function errorText(result: any): string {
  const first = result?.content?.[0];
  return first?.type === "text" ? first.text : "failed";
}

export default function banshee(pi: ExtensionAPI) {
  pi.registerTool({
    name: "speak_status",
    label: "Banshee Speak",
    description:
      "Speak a short message aloud to the user, who is working eyes-free and not reading the screen. This spoken message is your reply to them, so do not also repeat it as written text; reserve written output for what must be read on screen, such as code, file paths, commands, URLs, and lists. Use it to say what you decided or finished; when you need an answer back, use ask_user instead, which speaks and listens in one step. Talk like a colleague in the room: natural, warm, and varied, never scripted. When you finish, say what got done and flag anything still pending, then hand back to the user in your own words each time. When an implementation is done, mention it is ready for review. Do not narrate routine steps or tool activity in between.",
    promptSnippet: "Speak a short spoken message aloud to the user.",
    promptGuidelines: [
      "The user is listening, not reading. Say it with speak_status instead of writing it out, and do not repeat spoken text as prose.",
      "Use ask_user, not speak_status, whenever you need an answer back.",
    ],
    // One call at a time, so statuses and questions reach the daemon in the
    // order the agent made them.
    executionMode: "sequential",
    parameters: Type.Object({
      text: Type.String({
        description:
          "One or two conversational sentences, as if speaking to a colleague. Refer to code, files, and identifiers by their spoken names, for example 'the hotkey listener' rather than a file path or function signature. Keep exact paths, code, URLs, and lists in your normal text output; they do not read well aloud.",
      }),
    }),
    async execute(_toolCallId, params, signal) {
      await callDaemon(BANSHEE_SPEAK, { text: params.text }, signal);
      // The text is already in the call arguments; echoing it doubles it in context
      return { content: [{ type: "text", text: "ok" }], details: {} };
    },
    renderCall(args, theme) {
      return new Text(
        theme.fg("toolTitle", theme.bold("speak ")) +
          theme.fg("text", args.text ?? ""),
        0,
        0,
      );
    },
    renderResult(result, _options, theme, context) {
      if (context.isError) {
        return new Text(theme.fg("error", errorText(result)), 0, 0);
      }
      return new Text(theme.fg("success", "♪"), 0, 0);
    },
  });

  pi.registerTool({
    name: "ask_user",
    label: "Banshee Ask",
    description:
      "Ask the user a question aloud and wait for their spoken answer. Use it when you need a decision or clarification: the question is spoken, the microphone opens once it finishes playing, and the transcribed reply comes back scoped to you. Ask one focused question per call; when you have several, ask the most important first and wait for the answer before asking the next, so the user is never holding multiple questions in their head. If another question is open or the user is dictating, yours waits its turn and plays once the microphone is free, which can take minutes. Returns empty text if the user stayed silent, and an error if the listening itself failed, so silence and a failed listen are never confused.",
    promptSnippet: "Ask the user a question aloud and wait for a spoken answer.",
    promptGuidelines: [
      "Ask one question per ask_user call and wait for the answer before asking the next.",
    ],
    executionMode: "sequential",
    parameters: Type.Object({
      question: Type.String({
        description:
          "One or two conversational sentences, as if asking a colleague. Refer to code, files, and identifiers by their spoken names rather than paths or signatures.",
      }),
      timeout_ms: Type.Optional(
        Type.Number({
          description:
            "How long to wait for the user to start answering, in milliseconds. It starts once your question has played, not while it waits its turn. Defaults to 30000, capped at 120000.",
        }),
      ),
    }),
    async execute(_toolCallId, params, signal) {
      const result = await callDaemon(BANSHEE_ASK_USER, params, signal);
      const answer = typeof result?.text === "string" ? result.text.trim() : "";
      return {
        content: [
          { type: "text", text: answer || "(no answer: the user stayed silent)" },
        ],
        details: { answer },
      };
    },
    renderCall(args, theme) {
      return new Text(
        theme.fg("toolTitle", theme.bold("ask ")) +
          theme.fg("text", args.question ?? ""),
        0,
        0,
      );
    },
    renderResult(result, _options, theme, context) {
      if (context.isError) {
        return new Text(theme.fg("error", errorText(result)), 0, 0);
      }
      if (context.isPartial) {
        return new Text(theme.fg("muted", "listening..."), 0, 0);
      }
      const answer = (result.details as { answer?: string })?.answer;
      if (!answer) {
        return new Text(theme.fg("warning", "no answer"), 0, 0);
      }
      return new Text(
        theme.fg("success", "> ") + theme.fg("accent", answer),
        0,
        0,
      );
    },
  });
}
