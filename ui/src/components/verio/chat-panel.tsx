import { useState, useRef, useEffect, type ReactNode } from "react";
import { Button, IconButton } from "@/components/ui/button";
import { IconCopy, IconSend, IconX } from "@/components/verio/icons";
import { cn } from "@/lib/utils";

export interface ChatMessage {
  id: string;
  senderUserId: string;
  senderName: string;
  text: string;
  timestampMs: number;
  isSelf: boolean;
}

export interface ChatPanelProps {
  messages: ChatMessage[];
  onSendMessage: (text: string) => void;
  onClose?: () => void;
  className?: string;
}

function formatChatTime(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

function renderMessageContent(content: string): ReactNode {
  const regex = /(`[^`]+`|https?:\/\/[^\s]+)/g;
  const parts = content.split(regex);

  return parts.map((part, idx) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
      const code = part.slice(1, -1);
      return (
        <code
          key={idx}
          className="rounded-md border border-border bg-[var(--bg-input)] px-1.5 py-0.5 font-mono text-11 text-text"
        >
          {code}
        </code>
      );
    }
    if (/^https?:\/\//i.test(part)) {
      return (
        <a
          key={idx}
          href={part}
          target="_blank"
          rel="noopener noreferrer"
          className="text-accent underline hover:text-accent-hi break-all"
        >
          {part}
        </a>
      );
    }
    return <span key={idx}>{part}</span>;
  });
}

export function ChatPanel({
  messages,
  onSendMessage,
  onClose,
  className,
}: ChatPanelProps) {
  const [text, setText] = useState("");
  const messagesEndRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length]);

  const handleSend = () => {
    const trimmed = text.trim();
    if (!trimmed || trimmed.length > 500) return;
    onSendMessage(trimmed);
    setText("");
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <aside
      aria-label="Room chat"
      className={cn(
        "glass-card flex h-full min-h-0 flex-col overflow-hidden rounded-[22px]",
        className,
      )}
    >
      {/* Header */}
      <div className="flex shrink-0 items-center justify-between border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="label-caps text-lavender">Room Chat</span>
          <span className="mono-num rounded-full border border-border bg-[var(--bg-input)] px-2 py-0.5 font-mono text-10 text-text-dim">
            Ephemeral
          </span>
        </div>
        {onClose && (
          <IconButton label="Close chat" variant="ghost" size="sm" onClick={onClose}>
            <IconX size={14} />
          </IconButton>
        )}
      </div>

      {/* Message List */}
      <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto p-3.5">
        {messages.length === 0 ? (
          <div className="flex flex-1 items-center justify-center p-5 text-center text-13 text-text-dim">
            No messages yet. Messages disappear when you leave the room.
          </div>
        ) : (
          messages.map((msg, i) => {
            const prev = messages[i - 1];
            const isSameSender = prev && prev.senderUserId === msg.senderUserId;
            const isWithinTime = prev && msg.timestampMs - prev.timestampMs < 120_000;
            const isGrouped = isSameSender && isWithinTime;

            return (
              <div
                key={msg.id}
                className={cn(
                  "group relative flex flex-col gap-0.5 rounded-xl px-3 py-1.5 transition-colors hover:bg-surface-2/60",
                  !isGrouped && i > 0 && "mt-1",
                )}
              >
                {!isGrouped && (
                  <div className="flex items-baseline justify-between gap-2">
                    <span
                      className={cn(
                        "text-13 font-medium",
                        msg.isSelf ? "text-accent" : "text-lavender",
                      )}
                    >
                      {msg.isSelf ? "You" : msg.senderName}
                    </span>
                    <span className="mono-num font-mono text-10 text-text-dim">
                      {formatChatTime(msg.timestampMs)}
                    </span>
                  </div>
                )}
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0 flex-1 select-text whitespace-pre-wrap break-words text-13 leading-[1.45] text-text">
                    {renderMessageContent(msg.text)}
                  </div>
                  <IconButton
                    label="Copy message"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      void navigator.clipboard.writeText(msg.text);
                    }}
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                  >
                    <IconCopy size={14} />
                  </IconButton>
                </div>
              </div>
            );
          })
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Composer Footer */}
      <div className="flex shrink-0 flex-col gap-2 border-t border-border bg-[rgba(14,10,26,0.38)] p-3.5">
        <div className="flex items-center justify-between font-mono text-10 mono-num">
          <span className="text-text-dim">In-call chat</span>
          {text.length > 0 && (
            <span
              className={cn(text.length > 450 ? "text-warn" : "text-text-dim")}
            >
              {text.length}/500
            </span>
          )}
        </div>
        <div className="flex items-end gap-2">
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value.slice(0, 500))}
            onKeyDown={handleKeyDown}
            placeholder="Type a message..."
            rows={1}
            maxLength={500}
            className={cn(
              "max-h-24 min-h-9 flex-1 resize-none rounded-xl border border-border bg-[var(--bg-input)] px-3 py-2",
              "text-13 text-text placeholder:text-text-faint",
              "focus-visible:border-accent focus-visible:outline-none",
              "transition-colors",
            )}
            aria-label="Room chat message"
          />
          <Button
            variant="primary"
            size="md"
            disabled={!text.trim()}
            onClick={handleSend}
            icon={<IconSend size={14} />}
            aria-label="Send message"
            className="h-9 rounded-xl px-3.5"
          >
            Send
          </Button>
        </div>
      </div>
    </aside>
  );
}
