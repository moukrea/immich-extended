import { Show, type Component } from "solid-js";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

const ConfirmDialog: Component<ConfirmDialogProps> = (props) => {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/50 p-4"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
      >
        <div class="w-full max-w-md rounded-lg bg-white p-5 shadow-xl">
          <h2
            id="confirm-dialog-title"
            class="text-lg font-semibold text-slate-900"
          >
            {props.title}
          </h2>
          <p class="mt-2 text-sm text-slate-600">{props.message}</p>
          <div class="mt-4 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => props.onCancel()}
              class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => props.onConfirm()}
              class={
                props.destructive
                  ? "rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white shadow hover:bg-red-500"
                  : "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow hover:bg-indigo-500"
              }
            >
              {props.confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default ConfirmDialog;
