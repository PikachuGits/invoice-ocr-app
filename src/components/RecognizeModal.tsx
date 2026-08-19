import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

interface ProgressPayload {
  status: "progress" | "success" | "error";
  message: string;
}

interface QueuePayload {
  total: number;
  done: number;
  current: string;
}

interface RecognizeModalProps {
  onClose: () => void;
  onSuccess: () => void;
}

function RecognizeModal({ onClose, onSuccess }: RecognizeModalProps) {
  const [dragging, setDragging] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState<ProgressPayload | null>(null);
  const [queue, setQueue] = useState<QueuePayload | null>(null);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fileCount, setFileCount] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "over" || p.type === "enter") {
          setDragging(true);
        } else if (p.type === "leave") {
          setDragging(false);
        } else if (p.type === "drop") {
          setDragging(false);
          if (p.paths.length > 0) {
            recognizeFiles(p.paths);
          }
        }
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<ProgressPayload>("ocr-progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<QueuePayload>("ocr-queue", (event) => {
      setQueue(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const recognizeFiles = async (paths: string[]) => {
    if (processing || paths.length === 0) return;
    setProcessing(true);
    setDone(false);
    setError(null);
    setQueue(null);
    setFileCount(paths.length);
    setProgress({
      status: "progress",
      message: `正在提交 ${paths.length} 个文件...`,
    });
    try {
      await invoke("recognize_invoice", { imagePaths: paths });
      onSuccess();
      // 不自动关闭：显示完成状态，由用户确认
      setDone(true);
      setProcessing(false);
    } catch (err) {
      setError(String(err));
      setProcessing(false);
    }
  };

  const pickFile = async () => {
    if (processing) return;
    const selected = await open({
      multiple: true,
      filters: [
        { name: "发票文件", extensions: ["jpg", "jpeg", "png", "bmp", "webp", "pdf"] },
      ],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [String(selected)];
    recognizeFiles(paths);
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-recognize" onClick={(e) => e.stopPropagation()}>
        <h3>识别发票</h3>

        <div
          className={`drop-zone ${dragging ? "drop-active" : ""} ${
            processing ? "drop-disabled" : ""
          }`}
          onClick={processing ? undefined : pickFile}
        >
          <svg
            className="drop-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <polyline points="17 8 12 3 7 8" />
            <line x1="12" y1="3" x2="12" y2="15" />
          </svg>
          <p className="drop-title">
            {processing
              ? "识别中..."
              : done
              ? "识别完成，可继续拖入文件"
              : "将发票拖拽到此处"}
          </p>
          <p className="drop-sub">
            {processing
              ? `正在处理 ${fileCount} 个文件，请稍候...`
              : done
              ? "已识别完成，可再次拖拽/选择文件识别更多发票"
              : "支持多选/多文件拖拽（jpg / jpeg / png / bmp / webp / pdf），同发票号自动合并"}
          </p>
        </div>

        {queue && (
          <div className="queue-panel">
            <div className="queue-head">
              <span className="queue-title">排队进度</span>
              <span className="queue-count">
                {queue.done}/{queue.total} 个文件
              </span>
            </div>
            <div className="queue-bar">
              <div
                className="queue-bar-fill"
                style={{
                  width: `${Math.min(100, (queue.done / queue.total) * 100)}%`,
                }}
              />
            </div>
            {queue.current && (
              <div className="queue-current" title={queue.current}>
                正在识别: {queue.current}
              </div>
            )}
          </div>
        )}

        {progress && (
          <div className={`progress-bar progress-${progress.status}`}>
            {progress.status === "progress" && <span className="spinner" />}
            {progress.status === "success" && <span className="icon-check" />}
            {progress.status === "error" && <span className="icon-error" />}
            <span>{progress.message}</span>
          </div>
        )}

        {error && (
          <div className="progress-bar progress-error">
            <span className="error-text">{error}</span>
          </div>
        )}

        <div className="modal-actions">
          {done ? (
            <button className="btn btn-primary" onClick={onClose}>
              完成
            </button>
          ) : (
            <button className="btn btn-secondary" onClick={onClose}>
              {processing ? "取消（后台继续）" : "关闭"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

export default RecognizeModal;