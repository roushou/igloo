import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, type Change, type Comment } from "@/api/client";
import { CliCommand } from "@/components/action-button";
import { Empty } from "@/components/page";
import { Button } from "@/components/ui/button";
import { changeCli } from "@/lib/cli";
import { format } from "@/lib/format";
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
    <article className="rounded-lg border bg-card px-4 py-3 text-sm">
      <header className="mb-1 flex flex-wrap items-center gap-x-3 text-xs text-muted-foreground">
        <span className="font-medium text-foreground">{comment.author}</span>
        <time>{format.time(comment.at)}</time>
        <span>revision {comment.revision}</span>
        {comment.path ? (
          <span className="font-mono">
            {comment.path}
            {comment.line ? `:${comment.line}` : ""}
          </span>
        ) : null}
      </header>
      <p className="whitespace-pre-wrap">{comment.body}</p>
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
  const send = () =>
    comment.mutate(
      { body: body.trim(), revision, ...where },
      {
        onSuccess: () => {
          setBody("");
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
      <textarea
        aria-label={`${label} text`}
        rows={3}
        className="rounded-md border bg-card px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        placeholder="Write a comment"
        value={body}
        onChange={(event) => setBody(event.target.value)}
      />
      {comment.isError ? (
        <p role="alert" className="text-sm text-failed">
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
    <div className="flex max-w-3xl flex-col gap-4">
      {comments.length === 0 ? (
        <Empty>No comments yet.</Empty>
      ) : (
        <div className="flex flex-col gap-3">
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
