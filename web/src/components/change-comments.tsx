import { useMutation, useQueryClient } from "@tanstack/react-query";
import { MessageSquare } from "lucide-react";
import { useState } from "react";
import { api, type Change, type Comment } from "@/api/client";
import { CliCommand } from "@/components/action-button";
import { EmptyState } from "@/components/empty-state";
import { RelativeTime } from "@/components/relative-time";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { changeCli } from "@/lib/cli";
import { queryKeys } from "@/lib/queries";

/** Sends a comment on the change and keeps the result as the change's data. */
export function useComment(change: Change) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (comment: { body: string; revision?: number; path?: string; line?: number }) =>
      api.comment(change.id, comment),
    onSuccess: async (updated) => {
      queryClient.setQueryData(queryKeys.change(change.id), updated);
      await queryClient.invalidateQueries({ queryKey: queryKeys.changes(change.repo) });
    },
  });
}

/** One comment: who, when, where, and what they said. */
export function CommentCard({ comment }: { comment: Comment }) {
  return (
    <article className="flex gap-3 text-base">
      <span
        aria-hidden
        className="flex size-6 shrink-0 items-center justify-center rounded-full bg-muted text-sm font-semibold uppercase"
      >
        {comment.author.charAt(0)}
      </span>
      <div className="min-w-0 flex-1">
        <header className="mb-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-sm text-muted-foreground">
          <span className="font-medium text-foreground">{comment.author}</span>
          <RelativeTime at={comment.at} />
          <span>revision {comment.revision}</span>
          {comment.path ? (
            <span className="font-mono">
              {comment.path}
              {comment.line ? `:${comment.line}` : ""}
            </span>
          ) : null}
        </header>
        <p className="whitespace-pre-wrap">{comment.body}</p>
      </div>
    </article>
  );
}

/** A box to write a comment in, sent for `revision` and, for a line comment, `path` and `line`. */
export function CommentForm({
  change,
  revision,
  where,
  label,
  onDone,
}: {
  change: Change;
  revision: number;
  where?: { path: string; line: number };
  label: string;
  onDone?: () => void;
}) {
  const [body, setBody] = useState("");
  const comment = useComment(change);
  const toast = useToast();
  const send = () =>
    comment.mutate(
      { body: body.trim(), revision, ...where },
      {
        onSuccess: () => {
          setBody("");
          toast.show({ title: "Comment sent" });
          onDone?.();
        },
      },
    );
  return (
    <form
      aria-label={label}
      className="flex flex-col gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <Textarea
        aria-label={`${label} text`}
        rows={3}
        placeholder="Write a comment"
        value={body}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && body.trim()) {
            event.preventDefault();
            send();
          }
        }}
      />
      {comment.isError ? (
        <p role="alert" className="text-base text-failed">
          {comment.error.message}
        </p>
      ) : null}
      {body.trim() ? (
        <CliCommand command={changeCli.comment(change.id, body.trim(), { revision, ...where })} />
      ) : null}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={!body.trim() || comment.isPending}>
          {comment.isPending ? "Sending…" : "Comment"}
        </Button>
        {onDone ? (
          <Button type="button" size="sm" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
        ) : null}
      </div>
    </form>
  );
}

/** The Comments tab: every comment of the change, and a box for a new one. */
export function CommentsTab({ change, revision }: { change: Change; revision: number }) {
  const comments = change.comments ?? [];
  return (
    <div className="flex max-w-3xl flex-col gap-6">
      {comments.length === 0 ? (
        <EmptyState icon={MessageSquare} title="No comments yet.">
          Comments on the change or on a line of the diff appear here, and go to the agent when you
          request changes.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-5">
          {comments.map((comment) => (
            <CommentCard
              key={`${comment.at}-${comment.author}-${comment.body}`}
              comment={comment}
            />
          ))}
        </div>
      )}
      {change.phase === "open" ? (
        <CommentForm change={change} revision={revision} label="Comment on the change" />
      ) : null}
    </div>
  );
}
