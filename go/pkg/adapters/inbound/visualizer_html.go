package inbound

import (
	"strings"
)

// RenderVisualizerHTML renders the complete interactive dark-mode AntV X6 application.
func RenderVisualizerHTML(defaultDomain, routePrefix string) string {
	tpl := `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OCTOPUS Workflow Visualizer (Go Engine)</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Fira+Code:wght@400;500;600&family=Inter:wght@300;400;500;600;700&display=swap" rel="stylesheet">
    <script src="https://cdn.jsdelivr.net/npm/@antv/x6@2.18.1/dist/index.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/dagre@0.8.5/dist/dagre.min.js"></script>
    <style>
        :root {
            --bg-dark: #0f172a;
            --bg-card: #1e293b;
            --bg-card-hover: #334155;
            --border: #334155;
            --primary: #6366f1;
            --primary-glow: rgba(99, 102, 241, 0.35);
            --success: #10b981;
            --warning: #f59e0b;
            --danger: #ef4444;
            --text-main: #f8fafc;
            --text-muted: #94a3b8;
        }

        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }

        body {
            font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
            background-color: var(--bg-dark);
            color: var(--text-main);
            height: 100vh;
            overflow: hidden;
            display: flex;
            flex-direction: column;
        }

        /* Header Bar */
        header {
            height: 56px;
            background: rgba(11, 15, 25, 0.90);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 0 18px;
            gap: 14px;
            z-index: 50;
            flex-shrink: 0;
        }

        .header-left {
            display: flex;
            align-items: center;
            gap: 10px;
            flex-shrink: 0;
        }

        .logo-badge {
            display: flex;
            align-items: center;
            gap: 6px;
            background: linear-gradient(135deg, rgba(99, 102, 241, 0.25) 0%, rgba(168, 85, 247, 0.25) 100%);
            border: 1px solid rgba(168, 85, 247, 0.4);
            padding: 5px 10px;
            border-radius: 8px;
            font-weight: 700;
            font-size: 13px;
            letter-spacing: 0.5px;
            color: #fff;
            box-shadow: 0 0 14px rgba(168, 85, 247, 0.25);
            user-select: none;
            cursor: pointer;
        }

        .header-divider {
            width: 1px;
            height: 22px;
            background: rgba(255, 255, 255, 0.1);
            margin: 0 2px;
        }

        .select-pill {
            background: #131c2e;
            border: 1px solid rgba(255, 255, 255, 0.14);
            color: #f8fafc;
            padding: 6px 12px;
            border-radius: 8px;
            font-size: 13px;
            font-weight: 600;
            outline: none;
            cursor: pointer;
            transition: all 0.2s ease;
            max-width: 200px;
        }
        .select-pill:hover {
            border-color: rgba(99, 102, 241, 0.6);
            background: #18243c;
        }
        .select-pill:focus {
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.35);
        }

        .select-pill-route {
            background: #0d1e38;
            border-color: rgba(56, 189, 248, 0.45);
            color: #38bdf8;
            font-weight: 500;
            max-width: 260px;
        }
        .select-pill-route:hover {
            border-color: #38bdf8;
            background: #11284c;
        }

        /* Top-Left Workflow Description Overlay Tile */
        .top-left-overlay-container {
            position: absolute;
            top: 16px;
            left: 24px;
            z-index: 38;
            display: flex;
            flex-direction: column;
            gap: 10px;
            pointer-events: none;
            max-width: 440px;
        }

        .wf-overlay-tile {
            pointer-events: auto;
            background: rgba(15, 23, 42, 0.88);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid rgba(56, 189, 248, 0.25);
            border-radius: 10px;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.55);
            min-width: 240px;
            overflow: hidden;
            transition: all 0.25s cubic-bezier(0.16, 1, 0.3, 1);
            user-select: none;
        }

        .wf-tile-header {
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 8px 12px;
            background: rgba(30, 41, 59, 0.5);
            border-bottom: 1px solid rgba(255, 255, 255, 0.06);
            cursor: pointer;
            gap: 10px;
            transition: background 0.15s ease;
        }
        .wf-tile-header:hover {
            background: rgba(30, 41, 59, 0.85);
        }

        .wf-tile-title {
            display: flex;
            align-items: center;
            gap: 7px;
            font-size: 12px;
            font-weight: 700;
            color: #7dd3fc;
            letter-spacing: 0.3px;
        }
        .wf-tile-icon {
            font-size: 13px;
        }

        .wf-tile-toggle-btn {
            background: rgba(255, 255, 255, 0.08);
            border: 1px solid rgba(255, 255, 255, 0.12);
            color: #94a3b8;
            border-radius: 4px;
            width: 20px;
            height: 20px;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 9px;
            cursor: pointer;
            transition: all 0.15s ease;
            padding: 0;
        }
        .wf-tile-toggle-btn:hover {
            background: rgba(56, 189, 248, 0.2);
            border-color: #38bdf8;
            color: #fff;
        }

        .wf-tile-body {
            padding: 10px 14px;
            font-size: 12px;
            color: #cbd5e1;
            line-height: 1.5;
            user-select: text;
            max-height: 320px;
            overflow-y: auto;
            transition: max-height 0.25s ease, padding 0.25s ease, opacity 0.2s ease;
            display: flex;
            flex-direction: column;
            gap: 7px;
        }

        .wf-tile-row {
            display: flex;
            align-items: flex-start;
            gap: 8px;
            font-size: 12px;
        }
        .wf-tile-label {
            font-size: 11px;
            font-weight: 700;
            color: #94a3b8;
            font-family: 'Fira Code', monospace;
            min-width: 80px;
            flex-shrink: 0;
        }
        .wf-tile-val {
            color: #e2e8f0;
            word-break: break-word;
            flex: 1;
            font-size: 12px;
        }
        .wf-tile-val-highlight {
            font-weight: 600;
            color: #7dd3fc;
            font-family: 'Fira Code', monospace;
        }
        .wf-tile-val-code {
            font-family: 'Fira Code', monospace;
            font-size: 11px;
            color: #38bdf8;
            background: rgba(56, 189, 248, 0.1);
            border: 1px solid rgba(56, 189, 248, 0.25);
            padding: 2px 7px;
            border-radius: 4px;
            display: inline-block;
        }

        .wf-overlay-tile.minimized {
            border-color: rgba(255, 255, 255, 0.12);
            box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
        }
        .wf-overlay-tile.minimized .wf-tile-header {
            border-bottom: none;
            background: transparent;
            padding: 6px 12px;
        }
        .wf-overlay-tile.minimized .wf-tile-header:hover {
            background: rgba(30, 41, 59, 0.6);
        }
        .wf-overlay-tile.minimized .wf-tile-body {
            display: none;
        }

        .header-center {
            flex: 1;
            max-width: 440px;
            min-width: 240px;
        }

        .search-bar {
            display: flex;
            align-items: center;
            background: #0f172a;
            border: 1px solid rgba(255, 255, 255, 0.14);
            border-radius: 8px;
            padding: 3px 4px 3px 12px;
            gap: 8px;
            transition: all 0.2s ease;
        }
        .search-bar:focus-within {
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.35);
            background: #131d33;
        }
        .search-icon {
            font-size: 12px;
            color: #64748b;
            flex-shrink: 0;
        }
        .search-input {
            flex: 1;
            background: transparent;
            border: none;
            outline: none;
            color: #fff;
            font-size: 12px;
            font-family: 'Fira Code', monospace;
            min-width: 0;
        }
        .search-input::placeholder {
            color: #64748b;
        }
        .search-btn {
            background: #4f46e5;
            border: none;
            color: #fff;
            padding: 5px 12px;
            border-radius: 6px;
            font-size: 12px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 4px;
            transition: background 0.15s ease;
            flex-shrink: 0;
        }
        .search-btn:hover {
            background: #4338ca;
        }

        .header-right {
            display: flex;
            align-items: center;
            gap: 8px;
            flex-shrink: 0;
        }

        .btn {
            background: rgba(255, 255, 255, 0.05);
            border: 1px solid rgba(255, 255, 255, 0.1);
            color: #cbd5e1;
            height: 32px;
            padding: 0 12px;
            border-radius: 7px;
            font-size: 12px;
            font-weight: 500;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 6px;
            text-decoration: none;
            transition: all 0.15s ease;
            user-select: none;
            white-space: nowrap;
        }

        .btn:hover:not(:disabled) {
            background: rgba(255, 255, 255, 0.1);
            border-color: rgba(255, 255, 255, 0.2);
            color: #fff;
        }

        .btn-track {
            background: rgba(16, 185, 129, 0.12);
            border-color: rgba(16, 185, 129, 0.35);
            color: #34d399;
        }
        .btn-track:hover:not(:disabled) {
            background: rgba(16, 185, 129, 0.22);
            border-color: #10b981;
            color: #6ee7b7;
        }

        .btn-active-track {
            background: rgba(16, 185, 129, 0.25) !important;
            border-color: #10b981 !important;
            box-shadow: 0 0 12px rgba(16, 185, 129, 0.4);
        }

        .btn:disabled {
            opacity: 0.35;
            cursor: not-allowed;
            background: rgba(255, 255, 255, 0.02) !important;
            border-color: rgba(255, 255, 255, 0.05) !important;
            color: #475569 !important;
        }

        .live-badge {
            display: flex;
            align-items: center;
            gap: 6px;
            font-size: 11px;
            font-weight: 600;
            padding: 4px 9px;
            border-radius: 999px;
            background: rgba(16, 185, 129, 0.15);
            color: #34d399;
            border: 1px solid rgba(16, 185, 129, 0.3);
        }

        .pulse-dot {
            width: 7px;
            height: 7px;
            background: #10b981;
            border-radius: 50%;
            animation: pulse 2s infinite;
        }

        @keyframes pulse {
            0% { box-shadow: 0 0 0 0 rgba(16, 185, 129, 0.7); }
            70% { box-shadow: 0 0 0 8px rgba(16, 185, 129, 0); }
            100% { box-shadow: 0 0 0 0 rgba(16, 185, 129, 0); }
        }

        /* Main Workspace */
        .workspace {
            flex: 1;
            display: flex;
            position: relative;
            overflow: hidden;
        }

        #graph-container {
            flex: 1;
            height: 100%;
            background: radial-gradient(circle, #1e293b 1px, transparent 1px);
            background-size: 24px 24px;
            background-color: #0b0f19;
        }

        /* Sidebar Inspector - Collapsible Sliding Drawer */
        .inspector-panel {
            position: absolute;
            right: 0;
            top: 0;
            bottom: 0;
            width: 380px;
            background: rgba(15, 23, 42, 0.95);
            backdrop-filter: blur(20px);
            border-left: 1px solid var(--border);
            padding: 24px;
            display: flex;
            flex-direction: column;
            gap: 20px;
            overflow-y: auto;
            transform: translateX(100%);
            transition: transform 0.35s cubic-bezier(0.16, 1, 0.3, 1);
            z-index: 40;
            box-shadow: -10px 0 40px rgba(0, 0, 0, 0.6);
        }

        .inspector-panel.open {
            transform: translateX(0);
        }

        .floating-inspector-tab {
            position: absolute;
            right: 0;
            top: 24px;
            z-index: 35;
            background: rgba(30, 41, 59, 0.92);
            backdrop-filter: blur(12px);
            border: 1px solid var(--border);
            border-right: none;
            border-radius: 10px 0 0 10px;
            padding: 8px 14px;
            color: #94a3b8;
            font-size: 12px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 6px;
            box-shadow: -4px 4px 16px rgba(0, 0, 0, 0.4);
            transition: all 0.2s ease;
        }
        .floating-inspector-tab:hover {
            color: #38bdf8;
            background: rgba(51, 65, 85, 0.95);
            transform: translateX(-4px);
        }

        .close-panel-btn {
            background: transparent;
            border: none;
            color: #94a3b8;
            font-size: 14px;
            cursor: pointer;
            padding: 4px 8px;
            border-radius: 6px;
            transition: all 0.2s ease;
            display: flex;
            align-items: center;
            justify-content: center;
        }
        .close-panel-btn:hover {
            color: #fff;
            background: rgba(255, 255, 255, 0.1);
        }

        .panel-header {
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding-bottom: 12px;
            border-bottom: 1px solid var(--border);
        }

        .panel-title {
            font-size: 15px;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
        }

        .tag {
            display: inline-block;
            padding: 3px 8px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
            text-transform: uppercase;
        }
        .tag-activity { background: rgba(99, 102, 241, 0.2); color: #818cf8; border: 1px solid rgba(99, 102, 241, 0.4); }
        .tag-human { background: rgba(245, 158, 11, 0.2); color: #fbbf24; border: 1px solid rgba(245, 158, 11, 0.4); }
        .tag-branch { background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); }

        .meta-field {
            display: flex;
            flex-direction: column;
            gap: 6px;
        }

        .meta-label {
            font-size: 12px;
            font-weight: 500;
            color: var(--text-muted);
        }

        .meta-value {
            background: #0f172a;
            padding: 10px 12px;
            border-radius: 8px;
            border: 1px solid var(--border);
            font-size: 13px;
            font-family: 'Fira Code', monospace;
            word-break: break-all;
        }

        /* Floating Toolbar */
        .floating-toolbar {
            position: absolute;
            bottom: 20px;
            left: 20px;
            background: rgba(15, 23, 42, 0.85);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 10px;
            padding: 4px;
            display: flex;
            align-items: center;
            gap: 4px;
            box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
            z-index: 30;
        }

        .tool-btn {
            background: transparent;
            border: none;
            color: #94a3b8;
            padding: 6px 10px;
            border-radius: 6px;
            font-size: 12px;
            font-weight: 500;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 5px;
            transition: all 0.15s ease;
        }

        .tool-btn:hover {
            color: #fff;
            background: rgba(255, 255, 255, 0.08);
        }

        /* Approval Action Card inside Inspector */
        .approval-card {
            background: rgba(245, 158, 11, 0.1);
            border: 1px solid rgba(245, 158, 11, 0.3);
            border-radius: 12px;
            padding: 16px;
            display: flex;
            flex-direction: column;
            gap: 12px;
        }

        .approval-btn-group {
            display: flex;
            gap: 8px;
        }

        .btn-approve {
            flex: 1;
            background: #10b981;
            border: none;
            color: #fff;
            padding: 10px;
            border-radius: 8px;
            font-weight: 600;
            cursor: pointer;
            transition: opacity 0.2s;
        }
        .btn-approve:hover { opacity: 0.9; }

        .btn-reject {
            flex: 1;
            background: #ef4444;
            border: none;
            color: #fff;
            padding: 10px;
            border-radius: 8px;
            font-weight: 600;
            cursor: pointer;
            transition: opacity 0.2s;
        }
        .btn-reject:hover { opacity: 0.9; }

        /* Jobs Modal Overlay & Container */
        .modal-backdrop {
            position: fixed;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: rgba(8, 12, 22, 0.80);
            backdrop-filter: blur(12px);
            -webkit-backdrop-filter: blur(12px);
            z-index: 999;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 24px;
            animation: fadeIn 0.2s ease;
        }

        @keyframes fadeIn {
            from { opacity: 0; }
            to { opacity: 1; }
        }

        /* Jobs Modal Styles */
        .jobs-modal-card {
            background: #0f172a;
            border: 1px solid rgba(255, 255, 255, 0.15);
            border-radius: 14px;
            width: 100%;
            max-width: 1100px;
            height: 80vh;
            max-height: 85vh;
            display: flex;
            flex-direction: column;
            box-shadow: 0 25px 60px rgba(0, 0, 0, 0.8), 0 0 25px rgba(99, 102, 241, 0.2);
            overflow: hidden;
            animation: slideUp 0.25s cubic-bezier(0.16, 1, 0.3, 1);
            margin: auto;
        }

        @keyframes slideUp {
            from { transform: translateY(20px); opacity: 0; }
            to { transform: translateY(0); opacity: 1; }
        }

        .jobs-modal-header {
            padding: 18px 24px;
            background: rgba(15, 23, 42, 0.95);
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 16px;
        }

        .jobs-modal-title {
            display: flex;
            align-items: center;
            gap: 10px;
            font-size: 16px;
            font-weight: 700;
            color: #f8fafc;
        }

        .jobs-count-pill {
            background: rgba(99, 102, 241, 0.2);
            border: 1px solid rgba(99, 102, 241, 0.4);
            color: #c7d2fe;
            padding: 2px 10px;
            border-radius: 999px;
            font-size: 12px;
            font-weight: 600;
        }

        .jobs-modal-controls {
            padding: 14px 24px;
            background: rgba(11, 15, 25, 0.85);
            border-bottom: 1px solid rgba(255, 255, 255, 0.06);
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 12px;
            flex-wrap: wrap;
        }

        .jobs-filter-group {
            display: flex;
            align-items: center;
            gap: 10px;
            flex: 1;
        }

        .jobs-search-input {
            flex: 1;
            min-width: 200px;
            background: #1e293b;
            border: 1px solid rgba(255, 255, 255, 0.12);
            border-radius: 8px;
            padding: 8px 14px;
            color: #fff;
            font-size: 13px;
            font-family: 'Fira Code', monospace;
            outline: none;
            transition: all 0.2s ease;
        }

        .jobs-search-input:focus {
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.3);
            background: #182234;
        }

        .jobs-filter-select {
            background: #1e293b;
            border: 1px solid rgba(255, 255, 255, 0.12);
            border-radius: 8px;
            padding: 8px 12px;
            color: #f8fafc;
            font-size: 12px;
            font-weight: 500;
            outline: none;
            cursor: pointer;
        }

        .jobs-modal-body {
            flex: 1;
            overflow-y: auto;
            padding: 0;
            background: #0b0f19;
        }

        .jobs-table {
            width: 100%;
            border-collapse: collapse;
            font-size: 12px;
            text-align: left;
        }

        .jobs-table th {
            position: sticky;
            top: 0;
            background: #131c2e;
            color: #94a3b8;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            font-size: 11px;
            padding: 12px 16px;
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            z-index: 5;
        }

        .jobs-table td {
            padding: 12px 16px;
            border-bottom: 1px solid rgba(255, 255, 255, 0.05);
            vertical-align: middle;
        }

        .jobs-table tr:hover td {
            background: rgba(255, 255, 255, 0.03);
        }

        .job-id-code {
            font-family: 'Fira Code', monospace;
            font-size: 12px;
            color: #67e8f9;
            cursor: pointer;
            font-weight: 500;
        }
        .job-id-code:hover {
            text-decoration: underline;
            color: #a5f3fc;
        }

        .method-badge {
            display: inline-block;
            padding: 2px 6px;
            border-radius: 4px;
            font-size: 10px;
            font-weight: 700;
            font-family: 'Fira Code', monospace;
        }
        .method-post { background: rgba(168, 85, 247, 0.2); color: #d8b4fe; border: 1px solid rgba(168, 85, 247, 0.4); }
        .method-get { background: rgba(16, 185, 129, 0.2); color: #6ee7b7; border: 1px solid rgba(16, 185, 129, 0.4); }
        .method-put { background: rgba(245, 158, 11, 0.2); color: #fde68a; border: 1px solid rgba(245, 158, 11, 0.4); }
        .method-delete { background: rgba(239, 68, 68, 0.2); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.4); }

        .status-badge {
            display: inline-flex;
            align-items: center;
            gap: 5px;
            padding: 3px 9px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
        }
        .status-200 { background: rgba(16, 185, 129, 0.18); color: #34d399; border: 1px solid rgba(16, 185, 129, 0.35); }
        .status-401 { background: rgba(239, 68, 68, 0.18); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.35); }
        .status-403 { background: rgba(239, 68, 68, 0.18); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.35); }
        .status-429 { background: rgba(245, 158, 11, 0.18); color: #fde68a; border: 1px solid rgba(245, 158, 11, 0.35); }
        .status-500 { background: rgba(239, 68, 68, 0.22); color: #f87171; border: 1px solid rgba(239, 68, 68, 0.4); }
        .status-running { background: rgba(56, 189, 248, 0.18); color: #7dd3fc; border: 1px solid rgba(56, 189, 248, 0.35); }
        .status-rollback { background: rgba(168, 85, 247, 0.18); color: #d8b4fe; border: 1px solid rgba(168, 85, 247, 0.35); }

        .btn-inspect {
            background: rgba(99, 102, 241, 0.2);
            border: 1px solid rgba(99, 102, 241, 0.4);
            color: #c7d2fe;
            padding: 5px 12px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
            cursor: pointer;
            display: inline-flex;
            align-items: center;
            gap: 4px;
            transition: all 0.15s ease;
        }
        .btn-inspect:hover {
            background: rgba(99, 102, 241, 0.35);
            color: #fff;
            border-color: rgba(99, 102, 241, 0.6);
        }
    </style>
</head>
<body>

    <header>
        <div class="header-left">
            <div class="logo-badge" onclick="resetToBlueprint()" title="OCTOPUS Workflow Engine">
                <span>🐙</span>
                <span>OCTOPUS</span>
            </div>
            <div class="header-divider"></div>
            <select class="select-pill" id="domainSelect" onchange="onDomainSelectChanged()" title="Select Domain / Workflow"></select>
            <select class="select-pill select-pill-route" id="routeSelect" style="display: none;" onchange="onRouteSelectChanged()" title="Select API Route"></select>
        </div>

        <div class="header-center">
            <div class="search-bar">
                <span class="search-icon">🔍</span>
                <input type="text" class="search-input" id="jobIdInput" placeholder="Enter Job ID (e.g. payout-go-live-777)..." onkeydown="if(event.key==='Enter') lookupJob()">
                <button class="search-btn" id="lookupBtn" onclick="lookupJob()">Lookup</button>
            </div>
        </div>

        <div class="header-right">
            <button class="btn" onclick="openJobsModal()" title="View execution history & telemetry logs">
                <span>📑</span>
                <span>Jobs</span>
            </button>
            <button class="btn btn-track" id="trackBtn" onclick="toggleLiveTracking()" disabled title="Stream live execution events">
                <span>⚡</span>
                <span>Live Trace</span>
            </button>
            <button class="btn" onclick="resetToBlueprint()" title="Reset to static blueprint view">
                <span>↺</span>
                <span>Blueprint</span>
            </button>
            <a href="/docs" target="_blank" class="btn" style="color: #a5b4fc;" title="Open API documentation">
                <span>📚</span>
                <span>API Docs</span>
            </a>
            <div class="live-badge" id="liveIndicator" style="display: none;">
                <div class="pulse-dot"></div>
                <span>POLLING</span>
            </div>
        </div>
    </header>

    <!-- Floating Toast Notification -->
    <div id="toastMessage" style="display: none; position: fixed; top: 76px; right: 24px; z-index: 99; background: rgba(30, 41, 59, 0.95); backdrop-filter: blur(16px); border-radius: 10px; padding: 12px 18px; font-size: 13px; color: #fff; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5); align-items: center; gap: 10px; transition: all 0.3s ease;">
        <span id="toastIcon" style="font-size: 16px;">ℹ️</span>
        <span id="toastText">Notification message</span>
    </div>

    <div class="workspace">
        <div class="top-left-overlay-container">
            <div id="executionBanner" style="display: none; pointer-events: auto; background: rgba(15, 23, 42, 0.92); backdrop-filter: blur(12px); border: 1px solid rgba(16, 185, 129, 0.4); border-radius: 10px; padding: 8px 16px; font-size: 13px; color: #e2e8f0; box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5); align-items: center; gap: 8px; width: fit-content;">
                <span id="executionSummaryText">Execution Path Active</span>
            </div>

            <!-- Top-Left Minimizable Workflow Description Tile -->
            <div id="workflowTile" class="wf-overlay-tile">
                <div class="wf-tile-header" onclick="toggleWorkflowTile()" title="Click to minimize or expand workflow info">
                    <div class="wf-tile-title">
                        <span class="wf-tile-icon">📜</span>
                        <span id="wf-tile-name">Workflow Overview</span>
                    </div>
                    <button class="wf-tile-toggle-btn" id="wfTileToggleBtn" title="Minimize / Expand description">
                        <span id="wfTileToggleIcon">▲</span>
                    </button>
                </div>
                <div class="wf-tile-body" id="wfTileBody">
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">name:</span>
                        <span class="wf-tile-val wf-tile-val-highlight" id="wf-tile-name-val">-</span>
                    </div>
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">version:</span>
                        <span class="wf-tile-val" id="wf-tile-version-val">-</span>
                    </div>
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">description:</span>
                        <span class="wf-tile-val" id="wf-tile-desc-val">-</span>
                    </div>
                    <div class="wf-tile-row" id="wf-tile-route-row" style="display: none;">
                        <span class="wf-tile-label">route:</span>
                        <span class="wf-tile-val wf-tile-val-code" id="wf-tile-route-val">-</span>
                    </div>
                </div>
            </div>
        </div>

        <div id="graph-container"></div>

        <!-- Floating Button to Re-open Inspector when Folded -->
        <button id="openInspectorBtn" class="floating-inspector-tab" onclick="toggleInspector(true)" style="display: none;">
            <span>📋 Step Inspector</span>
        </button>

        <div class="inspector-panel" id="inspector">
            <div class="panel-header">
                <div class="panel-title">Step Inspector</div>
                <div style="display: flex; align-items: center; gap: 8px;">
                    <span class="tag tag-activity" id="insp-type">ACTIVITY</span>
                    <button class="close-panel-btn" onclick="toggleInspector(false)" title="Fold Inspector (Hide to Right)">✕</button>
                </div>
            </div>

            <div class="meta-field">
                <div class="meta-label">Step Identifier</div>
                <div class="meta-value" id="insp-id">-</div>
            </div>

            <div class="meta-field">
                <div class="meta-label">Execution Status</div>
                <div class="meta-value" id="insp-status" style="color: #38bdf8;">PENDING</div>
            </div>

            <div class="meta-field" id="insp-roles-field" style="display: none;">
                <div class="meta-label">Authorized Reviewer Roles</div>
                <div class="meta-value" id="insp-roles">-</div>
            </div>

            <!-- Security Policy Card -->
            <div class="meta-field" id="insp-policy-field" style="display: none; flex-direction: column; gap: 6px; background: rgba(15, 23, 42, 0.6); border: 1px solid rgba(251, 191, 36, 0.2); border-radius: 8px; padding: 10px;">
                <div class="meta-label" style="color: #fbbf24; font-weight: 600;">🛡️ Security Policy</div>
                <div id="insp-policy-badge"></div>
            </div>

            <!-- Ingress Middlewares Card -->
            <div class="meta-field" id="insp-middlewares-field" style="display: none; flex-direction: column; gap: 8px; background: rgba(15, 23, 42, 0.7); border: 1px solid rgba(56, 189, 248, 0.3); border-radius: 8px; padding: 12px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="color: #38bdf8; font-weight: 700;">🛡️ Ingress Guards Pipeline</span>
                    <span id="insp-middlewares-count-badge" style="background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #7dd3fc; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;"></span>
                </div>
                <div id="insp-middlewares-list" style="display: flex; flex-direction: column; gap: 6px; font-size: 11px;"></div>
                <div id="insp-middlewares-desc" style="font-size: 11px; color: var(--text-muted); line-height: 1.5; border-top: 1px dashed rgba(255,255,255,0.1); padding-top: 6px; margin-top: 2px;"></div>
            </div>

            <!-- Exception Handler & Error Routing Card -->
            <div class="meta-field" id="insp-exception-field" style="display: none; flex-direction: column; gap: 8px; background: rgba(15, 23, 42, 0.7); border: 1px solid rgba(239, 68, 68, 0.35); border-radius: 8px; padding: 12px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="color: #fca5a5; font-weight: 700;">🚨 Exception Handler Details</span>
                    <span id="insp-exc-status-badge" style="background: rgba(239, 68, 68, 0.15); border: 1px solid #ef4444; color: #fca5a5; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;"></span>
                </div>
                <div style="display: flex; flex-direction: column; gap: 6px; font-size: 11px;">
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Triggering Exception & HTTP Code:</div>
                        <div style="color: #fca5a5; font-family: 'Fira Code', monospace; font-weight: 600;" id="insp-exc-name">-</div>
                    </div>
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Origin / Provenance:</div>
                        <div style="color: #38bdf8; font-family: 'Fira Code', monospace; font-weight: 600;" id="insp-exc-origin">-</div>
                    </div>
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Execution Mode:</div>
                        <div style="color: #cbd5e1; font-family: 'Fira Code', monospace;" id="insp-exc-mode">-</div>
                    </div>
                </div>
                <div id="insp-exc-desc" style="font-size: 11px; color: var(--text-muted); line-height: 1.5; border-top: 1px dashed rgba(255,255,255,0.1); padding-top: 6px; margin-top: 2px;"></div>
            </div>

            <!-- Input and Output Payloads -->
            <div class="meta-field" id="insp-input-payload-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">📥 Input Payload</div>
                <div class="meta-value" id="insp-input-payload" style="max-height: 180px; overflow-y: auto;"><pre id="insp-input-json-content" style="white-space: pre-wrap; word-break: break-all; margin: 0;"></pre></div>
            </div>

            <div class="meta-field" id="insp-output-payload-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">📤 Output Payload</div>
                <div class="meta-value" id="insp-output-payload" style="max-height: 180px; overflow-y: auto;"><pre id="insp-output-json-content" style="white-space: pre-wrap; word-break: break-all; margin: 0;"></pre></div>
            </div>

            <div class="meta-field">
                <div class="meta-label">SLA Timeout</div>
                <div class="meta-value" id="insp-timeout">30s</div>
            </div>

            <!-- Branch Condition & Rule Evaluation Card -->
            <div class="meta-field" id="insp-branch-field" style="display: none; flex-direction: column; gap: 8px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="font-weight: 700; color: #38bdf8;">⚏ Branch Condition & Routing</span>
                    <span id="insp-branch-verdict-badge"></span>
                </div>
                
                <div id="insp-branch-rule-box" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; background: #0f172a; padding: 10px; border-radius: 8px; border-left: 3px solid #38bdf8; line-height: 1.6;">
                    <div id="insp-branch-rules-content"></div>
                </div>

                <div id="insp-branch-eval-box" style="display: none; background: rgba(15, 23, 42, 0.9); border: 1px solid rgba(16, 185, 129, 0.4); border-radius: 8px; padding: 10px; font-size: 11px; font-family: 'Fira Code', monospace;">
                    <div style="font-weight: 700; color: #10b981; margin-bottom: 6px; display: flex; align-items: center; gap: 6px;">
                        <span>⚡ Live Path Evaluation</span>
                    </div>
                    <div id="insp-branch-eval-details" style="color: #cbd5e1; line-height: 1.6;"></div>
                </div>

                <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px; font-size: 11px;">
                    <div id="insp-branch-then-box" style="background: rgba(16, 185, 129, 0.08); border: 1px solid rgba(16, 185, 129, 0.3); border-radius: 6px; padding: 8px;">
                        <div style="font-weight: 700; color: #34d399; margin-bottom: 4px;">✓ TRUE (Match)</div>
                        <div id="insp-branch-then-steps" style="color: #a7f3d0; font-family: 'Fira Code', monospace; font-size: 10px;"></div>
                    </div>
                    <div id="insp-branch-else-box" style="background: rgba(56, 189, 248, 0.08); border: 1px solid rgba(56, 189, 248, 0.3); border-radius: 6px; padding: 8px;">
                        <div style="font-weight: 700; color: #7dd3fc; margin-bottom: 4px;">✕ FALSE (Else)</div>
                        <div id="insp-branch-else-steps" style="color: #bae6fd; font-family: 'Fira Code', monospace; font-size: 10px;"></div>
                    </div>
                </div>
            </div>

            <!-- Dynamic Loop & Batch Configuration Card -->
            <div class="meta-field" id="insp-loop-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Dynamic Loop & Batch Configuration</div>
                <div id="insp-loop-badge"></div>
                <div id="insp-loop-details" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; background: #0f172a; padding: 8px 10px; border-radius: 6px; border-left: 2px solid #38bdf8; line-height: 1.5;"></div>
            </div>

            <!-- Activity Implementation Card -->
            <div class="meta-field" id="insp-activity-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Go Activity Implementation</div>
                <div id="insp-activity-status"></div>
                <div id="insp-activity-meta" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all;"></div>
            </div>

            <!-- Saga Compensation Availability Card -->
            <div class="meta-field" id="insp-comp-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Saga Rollback Compensation</div>
                <div id="insp-comp-status"></div>
            </div>

            <!-- Idempotency Strategy Card -->
            <div class="meta-field" id="insp-idempotency-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Idempotency Strategy</div>
                <div id="insp-idempotency-badge"></div>
            </div>

            <!-- Waiting for a system event (await_signal): nothing to approve -->
            <div class="approval-card" id="signalWait" style="display: none;">
                <div style="font-size: 13px; font-weight: 600; color: #fbbf24;">⏳ Waiting for an event</div>
                <div style="font-size: 12px; color: var(--text-muted);">This step resumes when the system sends the <code id="signalName" style="color:#fde68a;"></code> signal (for example a provider callback), or when it times out. There is nothing to approve here.</div>
            </div>

            <!-- Approval Interaction Card -->
            <div class="approval-card" id="approvalActions" style="display: none;">
                <div style="font-size: 13px; font-weight: 600; color: #fbbf24;">⚡ Human Approval Action</div>
                <div style="font-size: 12px; color: var(--text-muted);">This step is currently waiting for your decision.</div>
                
                <input type="text" class="input-box" id="reviewerId" placeholder="Reviewer ID (e.g. director_bob)" value="director_bob" style="width: 100%;">
                <input type="text" class="input-box" id="reviewerRoles" placeholder="Reviewer Roles (e.g. finance.manager)" value="finance.manager" style="width: 100%;">
                
                <div class="approval-btn-group">
                    <button class="btn-approve" onclick="submitDecision('APPROVED')">✓ Approve</button>
                    <button class="btn-reject" onclick="submitDecision('REJECTED')">✕ Reject</button>
                </div>
            </div>
        </div>

        <!-- Floating Graph Controls -->
        <div class="floating-toolbar">
            <button class="tool-btn" onclick="graph.zoom(0.15)">🔍 Zoom In</button>
            <button class="tool-btn" onclick="graph.zoom(-0.15)">🔍 Zoom Out</button>
            <button class="tool-btn" onclick="graph.zoomToFit({ padding: 40 })">⤢ Fit Screen</button>
            <button class="tool-btn" onclick="toggleLayout()">⇄ Switch Orientation</button>
        </div>
    </div>

    <!-- Jobs History Modal -->
    <div id="jobsModalBackdrop" class="modal-backdrop" style="display: none;" onclick="closeJobsModal(event)">
        <div class="jobs-modal-card" onclick="event.stopPropagation()">
            <div class="jobs-modal-header">
                <div class="jobs-modal-title">
                    <span>📑</span>
                    <span>Execution Jobs History</span>
                    <span class="jobs-count-pill" id="jobsCountPill">0 Jobs</span>
                </div>
                <div style="display: flex; align-items: center; gap: 10px;">
                    <label style="display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-muted); cursor: pointer; user-select: none;">
                        <input type="checkbox" id="autoRefreshJobs" onchange="toggleAutoRefreshJobs()" checked>
                        <span>Auto-refresh (3s)</span>
                    </label>
                    <button class="btn" onclick="loadJobsList()" title="Refresh jobs list">
                        <span>🔄</span>
                        <span>Refresh</span>
                    </button>
                    <button class="close-panel-btn" onclick="closeJobsModal()" title="Close modal" style="width: 28px; height: 28px;">✕</button>
                </div>
            </div>

            <div class="jobs-modal-controls">
                <div class="jobs-filter-group">
                    <input type="text" class="jobs-search-input" id="jobsSearchInput" placeholder="🔍 Filter by Job ID, IP, Route, Error..." oninput="filterJobsTable()">
                    <select class="jobs-filter-select" id="jobsStatusFilter" onchange="filterJobsTable()">
                        <option value="ALL">All Statuses</option>
                        <option value="200">200 OK / Completed</option>
                        <option value="401">401 Unauthorized</option>
                        <option value="403">403 Forbidden</option>
                        <option value="429">429 Rate Limit</option>
                        <option value="500">500 / Failed</option>
                        <option value="RUNNING">Running</option>
                        <option value="ROLLED_BACK">Rolled Back</option>
                    </select>
                    <select class="jobs-filter-select" id="jobsWorkflowFilter" onchange="filterJobsTable()">
                        <option value="ALL">All Workflows</option>
                    </select>
                </div>
            </div>

            <div class="jobs-modal-body">
                <table class="jobs-table">
                    <thead>
                        <tr>
                            <th style="width: 140px;">Status</th>
                            <th>Job ID</th>
                            <th>Workflow</th>
                            <th>Route & Method</th>
                            <th>Source IP</th>
                            <th>Execution Time</th>
                            <th>Started</th>
                            <th style="text-align: right; width: 110px;">Action</th>
                        </tr>
                    </thead>
                    <tbody id="jobsTableBody">
                        <tr>
                            <td colspan="8" style="text-align: center; color: #64748b; padding: 40px 0;">
                                Loading jobs...
                            </td>
                        </tr>
                    </tbody>
                </table>
            </div>
        </div>
    </div>

    <script>
        // Clean any embedded HTTP basic auth credentials from document URL to prevent Fetch API TypeErrors
        try {
            if (window.location.username || window.location.password || window.location.href.includes('@')) {
                const cleanOrigin = window.location.protocol + '//' + window.location.host;
                const cleanUrl = cleanOrigin + window.location.pathname + window.location.search + window.location.hash;
                window.history.replaceState({}, document.title, cleanUrl);
            }
        } catch (e) {
            console.debug("Could not sanitize URL:", e);
        }

        function safeApiUrl(path) {
            try {
                const cleanOrigin = window.location.protocol + '//' + window.location.host;
                if (!path) return cleanOrigin;
                if (path.startsWith('http://') || path.startsWith('https://')) {
                    const parsed = new URL(path);
                    return cleanOrigin + parsed.pathname + parsed.search;
                }
                return cleanOrigin + (path.startsWith('/') ? path : '/' + path);
            } catch (e) {
                return path;
            }
        }
        function escapeHtml(str) {
            if (str === null || str === undefined) return '';
            return String(str)
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;')
                .replace(/'/g, '&#039;');
        }

        const INITIAL_DOMAIN = "{{DEFAULT_DOMAIN}}";
        const INITIAL_PREFIX = "{{DEFAULT_PREFIX}}";
        let currentDomain = INITIAL_DOMAIN;
        let currentPrefix = INITIAL_PREFIX;
        let currentLayoutDir = "TB";
        let currentGraphData = null;
        let livePollingTimer = null;
        let selectedNodeData = null;
        let toastTimer = null;

        function getApiHeaders(extraHeaders = {}) {
            const urlParams = new URLSearchParams(window.location.search);
            // The API key is never read from the URL or defaulted: set it with
            // localStorage.setItem('octopus_api_key', '<key>') when the engine has one.
            const key = localStorage.getItem('octopus_api_key') || '';
            const headers = { ...extraHeaders };
            if (key) {
                headers['X-API-Key'] = key;
            }
            return headers;
        }

        function showToast(text, icon = 'ℹ️', borderColor = '#6366f1', duration = 5000) {
            const toast = document.getElementById('toastMessage');
            document.getElementById('toastText').innerText = text;
            document.getElementById('toastIcon').innerText = icon;
            toast.style.border = '1px solid ' + borderColor;
            toast.style.display = 'flex';
            if (toastTimer) clearTimeout(toastTimer);
            toastTimer = setTimeout(() => {
                toast.style.display = 'none';
            }, duration);
        }

        const domainSelect = document.getElementById('domainSelect');
        const routeSelect = document.getElementById('routeSelect');
        let currentRoute = "";
        let currentMethod = "";

        function populateRouteDropdown(routes, selectedRoute, selectedMethod) {
            if (!routeSelect) return;
            routeSelect.innerHTML = '';
            if (!routes || !Array.isArray(routes) || routes.length === 0) {
                routeSelect.style.display = 'none';
                currentRoute = "";
                currentMethod = "";
                return;
            }

            routeSelect.style.display = 'block';
            routes.forEach(r => {
                const opt = document.createElement('option');
                const m = (r.method || 'POST').toUpperCase();
                const path = r.route || '';
                const desc = r.summary || (r.step_count ? (r.step_count + ' steps') : '');
                opt.value = path;
                opt.dataset.method = m;
                opt.text = '[' + m + '] ' + path + (desc ? ' - ' + desc : '');
                if (path === selectedRoute || (selectedRoute && (selectedRoute.endsWith(path) || path.endsWith(selectedRoute)))) {
                    opt.selected = true;
                }
                routeSelect.appendChild(opt);
            });

            if (routeSelect.selectedIndex < 0 && routeSelect.options.length > 0) {
                routeSelect.selectedIndex = 0;
            }
            const activeOpt = routeSelect.options[routeSelect.selectedIndex >= 0 ? routeSelect.selectedIndex : 0];
            if (activeOpt) {
                currentRoute = activeOpt.value;
                currentMethod = activeOpt.dataset.method || 'POST';
            }
        }

        function onRouteSelectChanged() {
            const opt = routeSelect.options[routeSelect.selectedIndex];
            if (!opt) return;
            currentRoute = opt.value;
            currentMethod = opt.dataset.method || 'POST';

            loadBlueprintGraph();
        }

        function populateDropdown(domains) {
            domainSelect.innerHTML = '';
            if (!domains || !Array.isArray(domains) || domains.length === 0) {
                domains = [{
                    domain: INITIAL_DOMAIN || "payout_approval",
                    prefix: INITIAL_PREFIX || "/api/v1.0.0/payout_approval",
                    workflow_name: INITIAL_DOMAIN || "payout_approval"
                }];
            }
            domains.forEach(item => {
                const opt = document.createElement('option');
                opt.value = item.domain;
                opt.text = item.workflow_name || item.domain;
                opt.dataset.prefix = item.prefix;
                if (item.domain === currentDomain) opt.selected = true;
                domainSelect.appendChild(opt);
            });
            if (domainSelect.selectedIndex < 0 && domainSelect.options.length > 0) {
                domainSelect.selectedIndex = 0;
            }
            const activeOpt = domainSelect.options[domainSelect.selectedIndex >= 0 ? domainSelect.selectedIndex : 0];
            if (activeOpt) {
                currentDomain = activeOpt.value;
                currentPrefix = activeOpt.dataset.prefix;
            }
        }

        async function refreshDomains() {
            try {
                const res = await fetch(safeApiUrl('/api/visualizer/domains'), { headers: getApiHeaders() });
                if (res.ok) {
                    const domains = await res.json();
                    populateDropdown(domains);
                }
            } catch (e) {
                console.debug("Could not refresh domains list:", e);
            }
        }

        function onDomainSelectChanged() {
            const opt = domainSelect.options[domainSelect.selectedIndex];
            if (!opt) return;
            currentDomain = opt.value;
            currentPrefix = opt.dataset.prefix;

            const cleanOrigin = window.location.protocol + '//' + window.location.host;
            const newUrl = new URL(cleanOrigin + window.location.pathname + window.location.search);
            newUrl.searchParams.set('domain', currentDomain);
            window.history.pushState({}, '', newUrl.toString());

            document.getElementById('trackBtn').disabled = true;
            document.getElementById('trackBtn').classList.remove('btn-active-track');
            loadBlueprintGraph();
        }

        // Initialize AntV X6 Graph
        const graph = new X6.Graph({
            container: document.getElementById('graph-container'),
            autoResize: true,
            panning: true,
            mousewheel: {
                enabled: true,
                modifiers: ['ctrl', 'meta']
            },
            grid: {
                size: 20,
                visible: true,
                type: 'dot',
                args: { color: '#1e293b', thickness: 1 }
            },
            connecting: {
                router: {
                    name: 'manhattan',
                    args: {
                        padding: 10,
                        startDirections: ['bottom', 'right', 'left'],
                        endDirections: ['top', 'left', 'right'],
                        step: 10
                    }
                },
                connector: { name: 'rounded', args: { radius: 8 } },
                anchor: 'center',
                connectionPoint: 'boundary'
            }
        });
        window.graph = graph;

        // Register Node Shapes
        X6.Node.registry.register('octopus-activity-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 10,
                    ry: 10,
                    fill: '#1e293b',
                    stroke: '#4f46e5',
                    strokeWidth: 2
                },
                label: {
                    fill: '#f8fafc',
                    fontSize: 13,
                    fontWeight: 500,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-loop-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 12,
                    ry: 12,
                    fill: '#111827',
                    stroke: '#38bdf8',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 14px rgba(56, 189, 248, 0.35))'
                },
                label: {
                    fill: '#bae6fd',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-human-node', {
            inherit: 'polygon',
            attrs: {
                body: {
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#272015',
                    stroke: '#f59e0b',
                    strokeWidth: 2
                },
                label: {
                    fill: '#fde68a',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-quorum-approver-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 10,
                    ry: 10,
                    fill: '#1e293b',
                    stroke: '#3b82f6',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 10px rgba(59, 130, 246, 0.25))'
                },
                label: {
                    fill: '#93c5fd',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-quorum-join-node', {
            inherit: 'polygon',
            attrs: {
                body: {
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#134e4a',
                    stroke: '#14b8a6',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 12px rgba(20, 184, 166, 0.25))'
                },
                label: {
                    fill: '#5eead4',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-branch-node', {
            inherit: 'polygon',
            attrs: {
                body: {
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#241433',
                    stroke: '#a855f7',
                    strokeWidth: 2
                },
                label: {
                    fill: '#e9d5ff',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-start-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 25,
                    ry: 25,
                    fill: '#064e3b',
                    stroke: '#10b981',
                    strokeWidth: 2
                },
                label: {
                    fill: '#6ee7b7',
                    fontSize: 13,
                    fontWeight: 700
                }
            }
        });

        X6.Node.registry.register('octopus-rewind-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 10,
                    ry: 10,
                    fill: '#21170d',
                    stroke: '#f59e0b',
                    strokeWidth: 2,
                    strokeDasharray: '5 5'
                },
                label: {
                    fill: '#fde68a',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-jump-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 10,
                    ry: 10,
                    fill: '#082f49',
                    stroke: '#06b6d4',
                    strokeWidth: 2,
                    strokeDasharray: '5 5'
                },
                label: {
                    fill: '#67e8f9',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }
            }
        });

        X6.Node.registry.register('octopus-compensation-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 8,
                    ry: 8,
                    fill: '#1c1917',
                    stroke: '#f97316',
                    strokeWidth: 1.5,
                    strokeDasharray: '4 4'
                },
                label: {
                    fill: '#fdba74',
                    fontSize: 11,
                    fontWeight: 500
                }
            }
        });

        X6.Node.registry.register('octopus-end-node', {
            inherit: 'rect',
            attrs: {
                body: {
                    rx: 25,
                    ry: 25,
                    fill: '#450a0a',
                    stroke: '#ef4444',
                    strokeWidth: 2
                },
                label: {
                    fill: '#fca5a5',
                    fontSize: 13,
                    fontWeight: 700,
                    textWrap: {
                        width: -28,
                        ellipsis: true
                    }
                }
            }
        });

        // Handle Node Selection
        graph.on('node:click', ({ node }) => {
            const data = node.getData() || {};
            selectedNodeData = data;
            
            document.getElementById('insp-id').innerText = data.step_id || data.title || node.id;
            let typeLabel = (data.type || 'ACTIVITY').toUpperCase();
            if (data.is_sla_breach || data.type === 'escalation') {
                typeLabel = 'ESCALATION ACTIVITY (SLA BREACH)';
            } else if (data.is_async) {
                typeLabel = '⚡ ACTIVITY (ASYNC BACKGROUND)';
            } else if (data.type === 'compensation') {
                typeLabel = 'SAGA COMPENSATION';
            } else if (data.type === 'signal') {
                typeLabel = 'WAIT FOR SIGNAL';
            }
            document.getElementById('insp-type').innerText = typeLabel;
            document.getElementById('insp-status').innerText = data.status || 'PENDING';
            document.getElementById('insp-timeout').innerText = (data.timeout_seconds ? data.timeout_seconds + 's' : 'Default');

            // Status color & Compensation handling
            const statusEl = document.getElementById('insp-status');
            if (data.type === 'compensation') {
                if (data.status === 'ROLLED_BACK' || data.executed) {
                    statusEl.style.color = '#ea580c';
                    statusEl.innerText = 'ROLLED_BACK (Executed)';
                } else {
                    statusEl.style.color = '#f97316';
                    statusEl.innerText = 'STANDBY (Untriggered)';
                }
            } else if (data.is_sla_breach || data.type === 'escalation') {
                if (data.status === 'COMPLETED' || data.executed) {
                    statusEl.style.color = '#f97316';
                    statusEl.innerText = 'COMPLETED (Escalation)';
                } else {
                    statusEl.style.color = '#f97316';
                    statusEl.innerText = data.status || 'PENDING';
                }
            } else {
                if (data.status === 'COMPLETED' || data.status === 'BACKGROUND_DISPATCHED') statusEl.style.color = '#10b981';
                else if (data.status === 'AWAITING_APPROVAL' || data.status === 'AWAITING_SIGNAL') statusEl.style.color = '#fbbf24';
                else if (data.status === 'FAILED') statusEl.style.color = '#ef4444';
                else if (data.status === 'ROLLED_BACK') statusEl.style.color = '#ea580c';
                else if (data.status === 'SLA_BREACH' || data.status === 'TIMED_OUT') statusEl.style.color = '#f97316';
                else statusEl.style.color = '#38bdf8';
            }

            // Roles
            if (data.allowed_roles && data.allowed_roles.length > 0) {
                document.getElementById('insp-roles-field').style.display = 'flex';
                document.getElementById('insp-roles').innerText = data.allowed_roles.join(', ');
            } else {
                document.getElementById('insp-roles-field').style.display = 'none';
            }

            // Policy
            if (data.security_policy) {
                document.getElementById('insp-policy-field').style.display = 'flex';
                document.getElementById('insp-policy-badge').innerHTML = '<span style="color:#fbbf24; font-family:monospace;">' + escapeHtml(data.security_policy) + '</span>';
            } else {
                document.getElementById('insp-policy-field').style.display = 'none';
            }

            // Ingress Middlewares Card
            const mwField = document.getElementById('insp-middlewares-field');
            if (mwField) {
                if (data.middlewares && Array.isArray(data.middlewares) && data.middlewares.length > 0) {
                    mwField.style.display = 'flex';
                    document.getElementById('insp-middlewares-count-badge').innerText = data.middlewares.length + ' Active Guards';
                    const listEl = document.getElementById('insp-middlewares-list');
                    let listHtml = '';
                    data.middlewares.forEach((mw, i) => {
                        let mwName = typeof mw === 'object' ? (mw.name || JSON.stringify(mw)) : String(mw);
                        let mwDesc = '';
                        if (mwName.indexOf('BearerAuth') !== -1) mwDesc = 'JWT token validation, HMAC verification & claims identity injection';
                        else if (mwName.indexOf('RateLimit') !== -1) mwDesc = 'Quota enforcement (Burst token bucket & rate-limit window)';
                        else if (mwName.indexOf('IpWhitelist') !== -1 || mwName.indexOf('IpAccess') !== -1) mwDesc = 'Client IP subnet filtering & network access control';
                        else if (mwName.indexOf('CorrelationId') !== -1) mwDesc = 'Distributed tracing header propagation (X-Job-ID / Correlation)';
                        else mwDesc = 'Ingress filter & security policy guard';

                        listHtml += '<div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 6px 10px;">';
                        listHtml += '<div style="font-weight: 600; color: #38bdf8; display: flex; align-items: center; gap: 6px;">';
                        listHtml += '<span style="color: #94a3b8; font-size: 10px;">#' + (i+1) + '</span> <span>' + escapeHtml(mwName) + '</span>';
                        listHtml += '</div>';
                        if (mwDesc) {
                            listHtml += '<div style="font-size: 10px; color: #94a3b8; margin-top: 2px;">' + escapeHtml(mwDesc) + '</div>';
                        }
                        listHtml += '</div>';
                    });
                    listEl.innerHTML = listHtml;
                    const descEl = document.getElementById('insp-middlewares-desc');
                    descEl.innerText = data.description || 'Pre-execution security filters and quota validation guards executed before workflow pipeline.';
                } else {
                    mwField.style.display = 'none';
                }
            }

            // Exception Handler Card
            const excField = document.getElementById('insp-exception-field');
            if (excField) {
                if (data.type === 'exception_handler' || data.exception_type) {
                    excField.style.display = 'flex';
                    const code = data.status_code || 400;
                    const excType = data.exception_type || 'Exception';
                    document.getElementById('insp-exc-status-badge').innerText = 'HTTP ' + code;
                    document.getElementById('insp-exc-name').innerText = '⚠️ ' + excType + ' (' + code + ')';
                    
                    let originText = data.source_location || data.source_type || 'Workflow Blueprint Pipeline';
                    if (data.is_middleware || (!data.source_step_id && originText.indexOf('Middleware') !== -1)) {
                        originText = '🛡️ Middleware Exception Pipeline: ' + (data.middleware_name || data.source_name || 'Ingress Guards');
                    } else if (data.source_step_id) {
                        originText = '⚙️ Step Exception Handler: ' + data.source_step_id;
                    }
                    document.getElementById('insp-exc-origin').innerText = originText;

                    const isAsync = Boolean(data.is_async);
                    document.getElementById('insp-exc-mode').innerHTML = isAsync 
                        ? '<span style="color: #38bdf8;">⚡ Asynchronous</span> (Dispatched in background without blocking response)'
                        : '<span style="color: #fbbf24;">🔄 Synchronous</span> (Blocks until execution completes)';

                    document.getElementById('insp-exc-desc').innerText = data.description || 
                        ('Invoked upon ' + excType + ' to execute ' + (data.activity || 'exception recovery') + ' activity.');
                } else {
                    excField.style.display = 'none';
                }
            }

            // Dynamic Loop & Batch Configuration
            if (data.is_loop || data.type === 'loop' || Boolean(data.loop)) {
                document.getElementById('insp-loop-field').style.display = 'flex';
                const loopBadge = document.getElementById('insp-loop-badge');
                const loopDetails = document.getElementById('insp-loop-details');

                let loopType = 'Dynamic Loop Iteration';
                let detailsText = '';

                if (data.loop_for_each) {
                    loopType = '🔁 Collection Batching (for_each)';
                    detailsText = 'Target Collection: <span style="color: #38bdf8;">' + data.loop_for_each + '</span><br/>' +
                                  (data.loop_item_variable ? ('Item Variable: <span style="color: #a78bfa;">' + data.loop_item_variable + '</span><br/>') : '') +
                                  'Batch Execution: Sequential activity invocation per array element';
                } else if (data.loop_until) {
                    loopType = '⏱️ Polling Convergence (until)';
                    const u = data.loop_until;
                    const uVar = (typeof u === 'object' && u.variable) ? u.variable : 'status';
                    const uVal = (typeof u === 'object' && u.value !== undefined) ? JSON.stringify(u.value) : '"ACTIVE"';
                    detailsText = 'Target Condition: <span style="color: #38bdf8;">' + uVar + '</span> == <span style="color: #34d399;">' + uVal + '</span><br/>' +
                                  'Max Iterations: <span style="color: #fbbf24;">' + (data.loop_max_iterations || 5) + '</span><br/>' +
                                  'Interval: <span style="color: #fbbf24;">' + (data.loop_interval_seconds || 1) + 's</span><br/>' +
                                  'Backoff Factor: <span style="color: #fbbf24;">' + (data.loop_backoff_coefficient || 1.0) + 'x</span>';
                } else if (data.loop_while) {
                    loopType = '🔄 Condition Loop (while)';
                    const w = data.loop_while;
                    const wVar = (typeof w === 'object' && w.variable) ? w.variable : 'status';
                    const wVal = (typeof w === 'object' && w.value !== undefined) ? JSON.stringify(w.value) : '""';
                    detailsText = 'Continue While: <span style="color: #38bdf8;">' + wVar + '</span> == <span style="color: #34d399;">' + wVal + '</span><br/>' +
                                  'Max Iterations: <span style="color: #fbbf24;">' + (data.loop_max_iterations || 5) + '</span><br/>' +
                                  'Interval: <span style="color: #fbbf24;">' + (data.loop_interval_seconds || 1) + 's</span>';
                } else {
                    detailsText = 'Loop Configuration: ' + JSON.stringify(data.loop || {});
                }

                loopBadge.innerHTML = '<span style="background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #7dd3fc; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 700; display: inline-flex; align-items: center; gap: 5px;">' + loopType + '</span>';
                loopDetails.innerHTML = detailsText;
            } else {
                document.getElementById('insp-loop-field').style.display = 'none';
            }

            // Branch Conditions & Dynamic Rule Evaluation Card
            const branchField = document.getElementById('insp-branch-field');
            if (branchField) {
                const isBranch = data.type === 'branch' || Boolean(data.condition) || Boolean(data.conditions) || Boolean(data.branch_info);
                if (isBranch) {
                    branchField.style.display = 'flex';
                    const verdictBadge = document.getElementById('insp-branch-verdict-badge');
                    const rulesContent = document.getElementById('insp-branch-rules-content');
                    const evalBox = document.getElementById('insp-branch-eval-box');
                    const evalDetails = document.getElementById('insp-branch-eval-details');
                    const thenBox = document.getElementById('insp-branch-then-box');
                    const elseBox = document.getElementById('insp-branch-else-box');
                    const thenStepsEl = document.getElementById('insp-branch-then-steps');
                    const elseStepsEl = document.getElementById('insp-branch-else-steps');

                    const cond = data.condition || {};
                    const conds = data.conditions || {};
                    const bInfo = data.branch_info || {};

                    // 1. Build Rules Description
                    let rulesHtml = '';
                    if (cond.rule) {
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Rule:</strong> <span style="color: #fbbf24;">' + escapeHtml(cond.rule) + '</span></div>';
                    } else if (cond.variable || cond.field) {
                        const v = cond.variable || cond.field;
                        const op = cond.operator || '==';
                        const val = cond.value !== undefined ? JSON.stringify(cond.value) : '""';
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Expression:</strong> <span style="color: #38bdf8;">' + escapeHtml(v) + '</span> <span style="color: #f43f5e; font-weight: bold;">' + escapeHtml(op) + '</span> <span style="color: #34d399;">' + escapeHtml(val) + '</span></div>';
                    } else if (conds.conditions && Array.isArray(conds.conditions)) {
                        const matchMode = conds.match || 'all';
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Match Mode:</strong> <span style="color: #fbbf24;">' + matchMode.toUpperCase() + '</span> (' + conds.conditions.length + ' rules)</div>';
                        conds.conditions.forEach((sc, i) => {
                            const sv = sc.variable || sc.field || 'var';
                            const sop = sc.operator || '==';
                            const sval = sc.value !== undefined ? JSON.stringify(sc.value) : '""';
                            rulesHtml += '<div style="padding-left: 8px; border-left: 2px solid #334155; margin-top: 2px;">#' + (i+1) + ': <span style="color: #38bdf8;">' + escapeHtml(sv) + '</span> <span style="color: #f43f5e;">' + escapeHtml(sop) + '</span> <span style="color: #34d399;">' + escapeHtml(sval) + '</span></div>';
                        });
                    } else {
                        rulesHtml += '<div>Dynamic Decision Gate</div>';
                    }
                    rulesContent.innerHTML = rulesHtml;

                    // 2. Resolve Targets (Then / Else)
                    const thenList = (bInfo.then_step_ids && bInfo.then_step_ids.length > 0) ? bInfo.then_step_ids : (cond.then_steps ? cond.then_steps.map(s => s.id || s.activity) : []);
                    const elseList = (bInfo.else_step_ids && bInfo.else_step_ids.length > 0) ? bInfo.else_step_ids : (cond.else_steps ? cond.else_steps.map(s => s.id || s.activity) : []);
                    
                    thenStepsEl.innerText = thenList.length > 0 ? thenList.join(' ➔ ') : '(End / Next Step)';
                    elseStepsEl.innerText = elseList.length > 0 ? elseList.join(' ➔ ') : '(End / Next Step)';

                    // 3. Live Evaluation
                    const evalInfo = bInfo.evaluation || (data.evaluation);
                    const isExecuted = data.executed || (data.status === 'COMPLETED');

                    // Reset box styles
                    thenBox.style.boxShadow = 'none';
                    thenBox.style.borderColor = 'rgba(16, 185, 129, 0.3)';
                    elseBox.style.boxShadow = 'none';
                    elseBox.style.borderColor = 'rgba(56, 189, 248, 0.3)';

                    if (evalInfo && evalInfo.evaluated) {
                        evalBox.style.display = 'block';
                        const isTrue = evalInfo.verdict === 'TRUE';
                        if (isTrue) {
                            verdictBadge.innerHTML = '<span style="background: rgba(16, 185, 129, 0.2); border: 1px solid #10b981; color: #34d399; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✓ TRUE (Match)</span>';
                            thenBox.style.boxShadow = '0 0 12px rgba(16, 185, 129, 0.4)';
                            thenBox.style.borderColor = '#10b981';
                        } else {
                            verdictBadge.innerHTML = '<span style="background: rgba(56, 189, 248, 0.2); border: 1px solid #38bdf8; color: #7dd3fc; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✕ FALSE (Else)</span>';
                            elseBox.style.boxShadow = '0 0 12px rgba(56, 189, 248, 0.4)';
                            elseBox.style.borderColor = '#38bdf8';
                        }

                        let evalHtml = '<div><strong>Evaluated Variable:</strong> <span style="color: #38bdf8;">' + escapeHtml(evalInfo.variable_name || cond.variable || 'state') + '</span></div>';
                        if (evalInfo.runtime_value !== undefined) {
                            evalHtml += '<div><strong>Runtime Value:</strong> <span style="color: #34d399; font-weight: bold;">' + escapeHtml(JSON.stringify(evalInfo.runtime_value)) + '</span></div>';
                        }
                        if (evalInfo.operator && evalInfo.expected_value !== undefined) {
                            evalHtml += '<div><strong>Rule Check:</strong> <code>' + escapeHtml(JSON.stringify(evalInfo.runtime_value)) + ' ' + escapeHtml(evalInfo.operator) + ' ' + escapeHtml(JSON.stringify(evalInfo.expected_value)) + '</code> ➔ <strong style="color:' + (isTrue ? '#34d399' : '#38bdf8') + '">' + (isTrue ? 'MATCH (TRUE)' : 'MISMATCH (FALSE)') + '</strong></div>';
                        }
                        evalHtml += '<div style="margin-top: 4px; padding-top: 4px; border-top: 1px dashed rgba(255,255,255,0.1);"><strong>Active Branch Route:</strong> <span style="color:' + (isTrue ? '#34d399' : '#38bdf8') + '; font-weight: bold;">➔ ' + (isTrue ? 'TRUE (Then Branch)' : 'FALSE (Else Branch)') + '</span></div>';
                        evalDetails.innerHTML = evalHtml;
                    } else if (isExecuted) {
                        evalBox.style.display = 'block';
                        verdictBadge.innerHTML = '<span style="background: rgba(16, 185, 129, 0.2); border: 1px solid #10b981; color: #34d399; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✓ EXECUTED</span>';
                        evalDetails.innerHTML = '<div><strong>Status:</strong> <span style="color: #34d399;">Branch condition evaluated during workflow execution</span></div>';
                    } else {
                        evalBox.style.display = 'none';
                        verdictBadge.innerHTML = '<span style="background: rgba(100, 116, 139, 0.2); border: 1px solid #64748b; color: #94a3b8; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">STANDBY</span>';
                    }
                } else {
                    branchField.style.display = 'none';
                }
            }

            // Activity
            if (data.activity) {
                document.getElementById('insp-activity-field').style.display = 'flex';
                document.getElementById('insp-activity-status').innerHTML = '<span style="color:#34d399; font-weight:600;">✓ Registered Activity: ' + escapeHtml(data.activity) + '()</span>';
                document.getElementById('insp-activity-meta').innerText = '📁 activities (Go Engine)';
            } else {
                document.getElementById('insp-activity-field').style.display = 'none';
            }

            // Compensation
            if (data.compensation) {
                document.getElementById('insp-comp-field').style.display = 'flex';
                document.getElementById('insp-comp-status').innerHTML = '<span style="color:#fb923c; font-weight:600;">↩️ Rollback: ' + escapeHtml(data.compensation) + '()</span>';
            } else {
                document.getElementById('insp-comp-field').style.display = 'none';
            }

            // Idempotency
            if (data.idempotency_strategy) {
                document.getElementById('insp-idempotency-field').style.display = 'flex';
                document.getElementById('insp-idempotency-badge').innerHTML = '<span style="color:#38bdf8; font-family:monospace;">🔑 ' + escapeHtml(data.idempotency_strategy) + '</span>';
            } else {
                document.getElementById('insp-idempotency-field').style.display = 'none';
            }

            // Approval actions: only steps a person decides (human_step).
            // await_signal steps wait for a system event instead.
            const waiting = data.status === 'AWAITING_APPROVAL' || data.status === 'AWAITING_SIGNAL';
            document.getElementById('approvalActions').style.display = (waiting && data.type === 'human') ? 'flex' : 'none';
            document.getElementById('signalWait').style.display = (waiting && data.type === 'signal') ? 'flex' : 'none';
            document.getElementById('signalName').innerText = data.signal || '';

            // Input & Output Payload display
            const inputField = document.getElementById('insp-input-payload-field');
            const outputField = document.getElementById('insp-output-payload-field');
            if (data.input_payload && typeof data.input_payload === 'object' && Object.keys(data.input_payload).length > 0) {
                inputField.style.display = 'flex';
                document.getElementById('insp-input-json-content').innerText = JSON.stringify(data.input_payload, null, 2);
            } else {
                inputField.style.display = 'none';
            }
            if (data.output_payload && typeof data.output_payload === 'object' && Object.keys(data.output_payload).length > 0) {
                outputField.style.display = 'flex';
                document.getElementById('insp-output-json-content').innerText = JSON.stringify(data.output_payload, null, 2);
            } else {
                outputField.style.display = 'none';
            }

            toggleInspector(true);
        });

        graph.on('blank:click', () => {
            toggleInspector(false);
        });

        function toggleInspector(open = null) {
            const panel = document.getElementById('inspector');
            const openBtn = document.getElementById('openInspectorBtn');
            if (!panel) return;

            const isOpen = panel.classList.contains('open');
            const shouldOpen = open === null ? !isOpen : open;

            if (shouldOpen) {
                panel.classList.add('open');
                if (openBtn) openBtn.style.display = 'none';
            } else {
                panel.classList.remove('open');
                if (openBtn && selectedNodeData) openBtn.style.display = 'flex';
            }
        }

        window.selectNode = function(idOrMatch) {
            if (!graph || !graph.getNodes) return false;
            const nodes = graph.getNodes();
            const target = nodes.find(n => n.id === idOrMatch || n.id.includes(idOrMatch) || (n.getData() && (n.getData().step_id === idOrMatch || n.getData().activity === idOrMatch)));
            if (target) {
                graph.trigger('node:click', { node: target });
                return true;
            }
            return false;
        };

        // Auto-Layout using Dagre algorithm
        function applyDagreLayout(graphData, rankdir = 'TB') {
            if (!graph || typeof dagre === 'undefined' || !graphData) return;
            currentGraphData = graphData;

            let rawNodes = graphData.nodes || [];
            let rawEdges = graphData.edges || [];

            // Defensive: Filter out any edges where source or target does not exist in rawNodes
            const nodeIds = new Set(rawNodes.map(n => n.id));
            rawEdges = rawEdges.filter(edge => {
                const src = edge.source ? (edge.source.cell || edge.source) : edge.source;
                const tgt = edge.target ? (edge.target.cell || edge.target) : edge.target;
                return nodeIds.has(src) && nodeIds.has(tgt);
            });

            const g = new dagre.graphlib.Graph();
            g.setGraph({
                rankdir: rankdir,
                nodesep: rankdir === 'LR' ? 45 : 55,
                ranksep: rankdir === 'LR' ? 85 : 75,
                marginx: 35,
                marginy: 35
            });
            g.setDefaultEdgeLabel(() => ({}));

            rawNodes.forEach(node => {
                const w = node.width || 220;
                const h = node.height || 75;
                g.setNode(node.id, { width: w, height: h });
            });

            rawEdges.forEach(edge => {
                const src = edge.source ? (edge.source.cell || edge.source) : edge.source;
                const tgt = edge.target ? (edge.target.cell || edge.target) : edge.target;
                if (src && tgt && nodeIds.has(src) && nodeIds.has(tgt)) {
                    g.setEdge(src, tgt);
                }
            });

            dagre.layout(g);

            const positionedNodes = rawNodes.map(node => {
                const pos = g.node(node.id);
                const w = node.width || 220;
                const h = node.height || 75;
                if (pos) {
                    return {
                        ...node,
                        x: pos.x - w / 2,
                        y: pos.y - h / 2
                    };
                }
                return node;
            });

            const formattedEdges = rawEdges.map(edge => {
                if (edge.id && edge.id.includes('rewind')) {
                    return {
                        ...edge,
                        router: {
                            name: 'manhattan',
                            args: {
                                padding: 25,
                                startDirections: ['left', 'bottom'],
                                endDirections: ['left']
                            }
                        },
                        connector: { name: 'rounded', args: { radius: 12 } }
                    };
                }
                return {
                    ...edge,
                    router: {
                        name: 'manhattan',
                        args: {
                            padding: 10,
                            startDirections: rankdir === 'LR' ? ['right', 'bottom'] : ['bottom', 'right', 'left'],
                            endDirections: rankdir === 'LR' ? ['left', 'top'] : ['top', 'left', 'right'],
                            step: 10
                        }
                    },
                    connector: { name: 'rounded', args: { radius: 8 } }
                };
            });

            graph.fromJSON({
                nodes: positionedNodes,
                edges: formattedEdges
            });

            setTimeout(() => {
                graph.zoomToFit({ padding: 40, maxScale: 1.1 });
            }, 50);
        }

        function toggleLayout() {
            currentLayoutDir = currentLayoutDir === 'TB' ? 'LR' : 'TB';
            if (currentGraphData) {
                applyDagreLayout(currentGraphData, currentLayoutDir);
            }
        }

        function toggleWorkflowTile(forceState) {
            const tile = document.getElementById('workflowTile');
            const icon = document.getElementById('wfTileToggleIcon');
            if (!tile) return;
            let isMin;
            if (typeof forceState === 'boolean') {
                isMin = forceState;
                tile.classList.toggle('minimized', isMin);
            } else {
                isMin = tile.classList.toggle('minimized');
            }
            if (icon) {
                icon.innerText = isMin ? '▼' : '▲';
            }
            try {
                localStorage.setItem('octopus_wf_tile_minimized', isMin ? '1' : '0');
            } catch(e) {}
        }

        function updateWorkflowTile(data) {
            if (!data) return;
            const headerNameEl = document.getElementById('wf-tile-name');
            const nameValEl = document.getElementById('wf-tile-name-val');
            const verValEl = document.getElementById('wf-tile-version-val');
            const descValEl = document.getElementById('wf-tile-desc-val');
            const routeRowEl = document.getElementById('wf-tile-route-row');
            const routeValEl = document.getElementById('wf-tile-route-val');

            const domainName = data.workflow_name || currentDomain || 'Workflow';
            let versionStr = data.version ? (String(data.version).startsWith('v') ? data.version : 'v' + data.version) : 'v1.0.0';

            if (headerNameEl) {
                headerNameEl.innerText = 'Workflow Overview';
            }
            if (nameValEl) {
                nameValEl.innerText = domainName;
            }
            if (verValEl) {
                verValEl.innerText = versionStr;
            }
            if (descValEl) {
                descValEl.innerText = data.description || 'Interactive DAG workflow blueprint and live execution trace visualizer.';
            }

            const actRoute = data.selected_route || currentRoute;
            const actMethod = data.selected_method || currentMethod;
            if (routeRowEl && routeValEl) {
                if (actRoute) {
                    routeRowEl.style.display = 'flex';
                    routeValEl.innerText = (actMethod ? actMethod + ' ' : '') + actRoute;
                } else {
                    routeRowEl.style.display = 'none';
                }
            }
        }

        // Restore minimization state if set
        try {
            if (localStorage.getItem('octopus_wf_tile_minimized') === '1') {
                setTimeout(() => toggleWorkflowTile(true), 10);
            }
        } catch(e) {}

        // Load Static Blueprint
        async function loadBlueprintGraph() {
            if (livePollingTimer) {
                clearInterval(livePollingTimer);
                livePollingTimer = null;
            }
            document.getElementById('liveIndicator').style.display = 'none';
            document.getElementById('executionBanner').style.display = 'none';
            selectedNodeData = null;
            toggleInspector(false);

            if (!currentPrefix && domainSelect && domainSelect.options.length > 0) {
                const selIdx = domainSelect.selectedIndex >= 0 ? domainSelect.selectedIndex : 0;
                const activeOpt = domainSelect.options[selIdx];
                if (activeOpt) {
                    currentDomain = activeOpt.value;
                    currentPrefix = activeOpt.dataset.prefix;
                }
            }
            if (!currentPrefix) {
                currentPrefix = INITIAL_PREFIX || "/api/v1.0.0/payout_approval";
            }

            try {
                let url = currentPrefix + '/diagram/x6';
                const qParams = [];
                if (currentRoute) qParams.push('route=' + encodeURIComponent(currentRoute));
                if (currentMethod) qParams.push('method=' + encodeURIComponent(currentMethod));
                if (qParams.length > 0) url += '?' + qParams.join('&');

                const res = await fetch(safeApiUrl(url), { headers: getApiHeaders() });
                if (!res.ok) throw new Error('HTTP ' + res.status);
                const data = await res.json();
                updateWorkflowTile(data);
                if (data.routes) {
                    populateRouteDropdown(data.routes, data.selected_route || currentRoute, data.selected_method || currentMethod);
                }
                applyDagreLayout(data, currentLayoutDir);
            } catch (err) {
                console.error("Failed to load blueprint:", err);
                showToast('Could not load blueprint: ' + err.message, '⚠️', '#f59e0b', 4000);
            }
        }

        function resetToBlueprint() {
            document.getElementById('jobIdInput').value = '';
            document.getElementById('trackBtn').disabled = true;
            document.getElementById('trackBtn').classList.remove('btn-active-track');
            loadBlueprintGraph();
        }

        async function lookupJob() {
            const jobId = document.getElementById('jobIdInput').value.trim();
            if (!jobId) {
                showToast('Please enter a valid Job ID to lookup.', '⚠️', '#f59e0b');
                return;
            }

            try {
                let targetPrefix = currentPrefix;
                let targetJobId = jobId;
                let childInfo = null;

                // 1. Check resolve endpoint to verify domain
                try {
                    const resolveRes = await fetch(safeApiUrl('/api/visualizer/resolve-job?job_id=' + encodeURIComponent(jobId)), { headers: getApiHeaders() });
                    if (resolveRes.ok) {
                        const resolved = await resolveRes.json();
                        if (resolved.exists) {
                            if (resolved.target_job_id && resolved.target_job_id !== jobId) {
                                targetJobId = resolved.target_job_id;
                                childInfo = resolved;
                                showToast('🔗 Linked child workflow to parent ' + resolved.target_job_id, '🔗', '#38bdf8');
                            }
                            if (resolved.route) {
                                currentRoute = resolved.route;
                            }
                            if (resolved.method) {
                                currentMethod = resolved.method;
                            }
                            if (resolved.domain && resolved.prefix) {
                                targetPrefix = resolved.prefix;
                                currentDomain = resolved.domain;
                                currentPrefix = resolved.prefix;
                                const sel = document.getElementById('domainSelect');
                                if (sel) {
                                    for (let i = 0; i < sel.options.length; i++) {
                                        if (sel.options[i].value === resolved.domain) {
                                            sel.selectedIndex = i;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } catch (e) {
                    console.debug("Resolve job check skipped:", e);
                }

                let diagUrl = targetPrefix + '/diagram/x6?job_id=' + encodeURIComponent(targetJobId);
                if (currentRoute) {
                    diagUrl += '&route=' + encodeURIComponent(currentRoute);
                }
                if (currentMethod) {
                    diagUrl += '&method=' + encodeURIComponent(currentMethod);
                }
                const res = await fetch(safeApiUrl(diagUrl), { headers: getApiHeaders() });
                const data = await res.json();

                if (data.workflow_name && data.workflow_name !== currentDomain) {
                    const sel = document.getElementById('domainSelect');
                    if (sel) {
                        for (let i = 0; i < sel.options.length; i++) {
                            if (sel.options[i].value === data.workflow_name) {
                                sel.selectedIndex = i;
                                currentDomain = data.workflow_name;
                                currentPrefix = sel.options[i].dataset.prefix;
                                break;
                            }
                        }
                    }
                }
                updateWorkflowTile(data);
                if (data.routes) {
                    populateRouteDropdown(data.routes, data.selected_route, data.selected_method);
                    if (data.selected_route) {
                        currentRoute = data.selected_route;
                        currentMethod = data.selected_method;
                    }
                }

                applyDagreLayout(data, currentLayoutDir);

                const banner = document.getElementById('executionBanner');
                banner.style.display = 'flex';

                const status = (data.overall_status || data.status || '').toUpperCase();
                if (!status || status === 'IDLE') {
                    banner.style.background = 'rgba(45, 10, 10, 0.9)';
                    banner.style.borderColor = 'rgba(239, 68, 68, 0.6)';
                    banner.style.boxShadow = '0 4px 24px rgba(239, 68, 68, 0.25)';
                    document.getElementById('executionSummaryText').innerHTML = '⚠️ Job ID <strong>"' + escapeHtml(targetJobId) + '"</strong> Not Found. Showing Base Blueprint Diagram.';
                    showToast('Job ' + targetJobId + ' not found', '⚠️', '#ef4444');
                    return;
                }

                // Count executed compensations
                const rolledBackNodes = (data.nodes || []).filter(n => n.data && (n.data.status === 'ROLLED_BACK' || (n.data.type === 'compensation' && n.data.executed)));
                let compBadge = '';
                if (rolledBackNodes.length > 0) {
                    compBadge = '<span style="background: rgba(234, 88, 12, 0.25); color: #fdba74; border: 1px solid #ea580c; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">↩️ ' + rolledBackNodes.length + ' SAGA COMPENSATED</span>';
                }

                let icon = '✨';
                let statusBadge = '';
                let bannerBg = 'rgba(15, 23, 42, 0.9)';
                let bannerBorder = 'rgba(56, 189, 248, 0.4)';
                let bannerBoxShadow = '0 4px 20px rgba(0, 0, 0, 0.5)';

                if (status === 'ROLLED_BACK' || status === 'ROLLING_BACK' || rolledBackNodes.length > 0) {
                    icon = '↩️';
                    statusBadge = '<span style="background: rgba(239, 68, 68, 0.2); color: #fca5a5; border: 1px solid #ef4444; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">ROLLED_BACK</span>';
                    bannerBg = 'rgba(45, 10, 10, 0.9)';
                    bannerBorder = 'rgba(239, 68, 68, 0.6)';
                    bannerBoxShadow = '0 4px 24px rgba(239, 68, 68, 0.25)';
                } else if (status === 'FAILED') {
                    icon = '❌';
                    statusBadge = '<span style="background: rgba(239, 68, 68, 0.2); color: #fca5a5; border: 1px solid #ef4444; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">FAILED</span>';
                    bannerBg = 'rgba(45, 10, 10, 0.9)';
                    bannerBorder = 'rgba(239, 68, 68, 0.6)';
                    bannerBoxShadow = '0 4px 24px rgba(239, 68, 68, 0.25)';
                } else if (status === 'TIMED_OUT') {
                    icon = '⏳';
                    statusBadge = '<span style="background: rgba(249, 115, 22, 0.2); color: #fdba74; border: 1px solid #f97316; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">TIMED OUT</span>';
                    bannerBg = 'rgba(45, 26, 3, 0.9)';
                    bannerBorder = 'rgba(249, 115, 22, 0.6)';
                    bannerBoxShadow = '0 4px 24px rgba(249, 115, 22, 0.25)';
                } else if (status === 'COMPLETED') {
                    icon = '✅';
                    statusBadge = '<span style="background: rgba(16, 185, 129, 0.2); color: #6ee7b7; border: 1px solid #10b981; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">COMPLETED</span>';
                    bannerBg = 'rgba(6, 40, 30, 0.9)';
                    bannerBorder = 'rgba(16, 185, 129, 0.5)';
                    bannerBoxShadow = '0 4px 24px rgba(16, 185, 129, 0.25)';
                } else if (status === 'AWAITING_APPROVAL' || status === 'AWAITING_SIGNAL' || status === 'RUNNING') {
                    icon = '⚡';
                    const waitLabel = status === 'AWAITING_APPROVAL' ? 'AWAITING REVIEW' : (status === 'AWAITING_SIGNAL' ? 'WAITING FOR SIGNAL' : 'RUNNING');
                    statusBadge = '<span style="background: rgba(245, 158, 11, 0.2); color: #fde68a; border: 1px solid #f59e0b; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">' + waitLabel + '</span>';
                    bannerBg = 'rgba(35, 25, 5, 0.9)';
                    bannerBorder = 'rgba(245, 158, 11, 0.6)';
                    bannerBoxShadow = '0 4px 24px rgba(245, 158, 11, 0.25)';
                } else if (status === 'TERMINATED' || status === 'CANCELED') {
                    icon = '⏹️';
                    statusBadge = '<span style="background: rgba(100, 116, 139, 0.2); color: #cbd5e1; border: 1px solid #64748b; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">' + status + '</span>';
                    bannerBg = 'rgba(15, 23, 42, 0.9)';
                    bannerBorder = 'rgba(100, 116, 139, 0.5)';
                }

                let childBadge = '';
                if (childInfo && childInfo.target_job_id && childInfo.target_job_id !== jobId) {
                    childBadge = '<span style="background: rgba(56, 189, 248, 0.2); color: #7dd3fc; border: 1px solid #38bdf8; padding: 2px 8px; border-radius: 4px; font-weight: 600; margin-left: 8px; font-size: 11px;">🔗 Child (' + escapeHtml(jobId) + ')</span>';
                }

                const execCount = (data.executed_count !== undefined) ? data.executed_count : (data.nodes ? data.nodes.filter(n => n.data && n.data.executed).length : 0);
                document.getElementById('executionSummaryText').innerHTML = '✨ Execution Path: <strong>' + execCount + ' steps executed</strong> • Status: ' + statusBadge + compBadge + childBadge;

                document.getElementById('trackBtn').disabled = false;
                showToast('Job ' + jobId + ' trace loaded successfully!', '✓', '#10b981');
            } catch (err) {
                showToast('Lookup failed: ' + err.message, '⚠️', '#ef4444');
            }
        }

        function toggleLiveTracking() {
            const trackBtn = document.getElementById('trackBtn');
            if (livePollingTimer) {
                clearInterval(livePollingTimer);
                livePollingTimer = null;
                trackBtn.classList.remove('btn-active-track');
                trackBtn.innerText = '⚡ Track Live';
                document.getElementById('liveIndicator').style.display = 'none';
            } else {
                trackBtn.classList.add('btn-active-track');
                trackBtn.innerText = '⏹ Stop Tracking';
                document.getElementById('liveIndicator').style.display = 'flex';
                livePollingTimer = setInterval(lookupJob, 2500);
            }
        }

        async function submitDecision(decision) {
            const jobId = document.getElementById('jobIdInput').value.trim() || document.getElementById('insp-id').innerText;
            const reviewerId = document.getElementById('reviewerId').value.trim();
            const reviewerRoles = document.getElementById('reviewerRoles').value.trim().split(',').map(s => s.trim());

            try {
                const res = await fetch(safeApiUrl(currentPrefix + '/signal/' + encodeURIComponent(jobId)), {
                    method: 'POST',
                    headers: getApiHeaders({ 'Content-Type': 'application/json', 'X-User-ID': reviewerId, 'X-User-Roles': reviewerRoles.join(',') }),
                    body: JSON.stringify({
                        decision: decision,
                        reviewed_by: reviewerId,
                        reviewer_roles: reviewerRoles,
                        reviewer_notes: 'Decision submitted from OCTOPUS Visualizer'
                    })
                });

                if (!res.ok) {
                    const errJson = await res.json();
                    throw new Error(errJson.message || 'Signal rejected');
                }

                showToast('Decision ' + decision + ' recorded successfully!', '✓', '#10b981');
                setTimeout(lookupJob, 1000);
            } catch (err) {
                showToast('Approval submission failed: ' + err.message, '✕', '#ef4444');
            }
        }

        let allJobsData = [];
        let jobsAutoRefreshTimer = null;

        function openJobsModal() {
            document.getElementById('jobsModalBackdrop').style.display = 'flex';
            populateWorkflowFilterDropdown();
            loadJobsList();
            if (document.getElementById('autoRefreshJobs') && document.getElementById('autoRefreshJobs').checked && !jobsAutoRefreshTimer) {
                jobsAutoRefreshTimer = setInterval(loadJobsList, 3000);
            }
        }

        function closeJobsModal(e) {
            if (e && e.target && e.target.id !== 'jobsModalBackdrop') return;
            document.getElementById('jobsModalBackdrop').style.display = 'none';
            if (jobsAutoRefreshTimer) {
                clearInterval(jobsAutoRefreshTimer);
                jobsAutoRefreshTimer = null;
            }
        }

        function toggleAutoRefreshJobs() {
            const isChecked = document.getElementById('autoRefreshJobs').checked;
            if (isChecked) {
                if (!jobsAutoRefreshTimer) jobsAutoRefreshTimer = setInterval(loadJobsList, 3000);
            } else {
                if (jobsAutoRefreshTimer) {
                    clearInterval(jobsAutoRefreshTimer);
                    jobsAutoRefreshTimer = null;
                }
            }
        }

        async function populateWorkflowFilterDropdown() {
            const wfSelect = document.getElementById('jobsWorkflowFilter');
            if (!wfSelect) return;
            const currentVal = wfSelect.value;
            try {
                const res = await fetch(safeApiUrl('/api/visualizer/domains'), { headers: getApiHeaders() });
                if (res.ok) {
                    const domains = await res.json();
                    let html = '<option value="ALL">All Workflows</option>';
                    domains.forEach(d => {
                        html += '<option value="' + escapeHtml(d.domain) + '">' + escapeHtml(d.domain) + '</option>';
                    });
                    wfSelect.innerHTML = html;
                    if (currentVal) wfSelect.value = currentVal;
                }
            } catch (e) {
                console.debug('Failed to populate workflow filter', e);
            }
        }

        async function loadJobsList() {
            try {
                const res = await fetch(safeApiUrl('/api/visualizer/jobs?limit=150'), { headers: getApiHeaders() });
                if (!res.ok) throw new Error('Failed to fetch jobs');
                const data = await res.json();
                allJobsData = Array.isArray(data) ? data : [];
                filterJobsTable();
            } catch (err) {
                console.debug('Failed to load jobs list:', err);
            }
        }

        function filterJobsTable() {
            const search = (document.getElementById('jobsSearchInput').value || '').toLowerCase().trim();
            const statusFilter = document.getElementById('jobsStatusFilter').value;
            const wfFilter = document.getElementById('jobsWorkflowFilter').value;
            const tbody = document.getElementById('jobsTableBody');
            const countPill = document.getElementById('jobsCountPill');

            let filtered = allJobsData.filter(job => {
                if (wfFilter !== 'ALL' && job.workflow_name !== wfFilter && job.workflow !== wfFilter) {
                    return false;
                }

                const statusCode = job.status_code || 0;
                const statusStr = (job.status || '').toUpperCase();

                if (statusFilter === '200') {
                    if (statusCode !== 200 && statusStr !== 'COMPLETED') return false;
                } else if (statusFilter === '401') {
                    if (statusCode !== 401 && !statusStr.includes('401')) return false;
                } else if (statusFilter === '403') {
                    if (statusCode !== 403 && !statusStr.includes('403')) return false;
                } else if (statusFilter === '429') {
                    if (statusCode !== 429 && !statusStr.includes('429')) return false;
                } else if (statusFilter === '500') {
                    if (statusCode < 500 && statusStr !== 'FAILED') return false;
                } else if (statusFilter === 'RUNNING') {
                    if (statusStr !== 'RUNNING') return false;
                } else if (statusFilter === 'ROLLED_BACK') {
                    if (statusStr !== 'ROLLED_BACK') return false;
                }

                if (search) {
                    const match = (job.job_id || '').toLowerCase().includes(search)
                        || (job.workflow_name || '').toLowerCase().includes(search)
                        || (job.route || '').toLowerCase().includes(search)
                        || (job.method || '').toLowerCase().includes(search)
                        || (job.source_ip || '').toLowerCase().includes(search)
                        || (job.error || '').toLowerCase().includes(search)
                        || (job.error_type || '').toLowerCase().includes(search);
                    if (!match) return false;
                }

                return true;
            });

            if (countPill) {
                countPill.innerText = filtered.length + ' of ' + allJobsData.length + ' Jobs';
            }

            if (filtered.length === 0) {
                tbody.innerHTML = '<tr><td colspan="8" style="text-align: center; color: #64748b; padding: 40px 0;"><div style="font-size: 24px; margin-bottom: 8px;">📂</div><div>No matching executions found</div><div style="font-size: 11px; margin-top: 4px; color: #475569;">Send an API request or curl command to populate live execution history.</div></td></tr>';
                return;
            }

            let html = '';
            filtered.forEach(job => {
                const jId = escapeHtml(job.job_id || '');
                const wf = escapeHtml(job.workflow_name || job.workflow || '-');
                const route = escapeHtml(job.route || job.route_id || '-');
                const rawWf = job.workflow_name || job.workflow || '-';
                const rawRoute = job.route || job.route_id || '-';
                const method = escapeHtml((job.method || job.route_method || 'POST').toUpperCase());
                const ip = escapeHtml(job.source_ip || job.client_host || '127.0.0.1');
                const duration = job.duration_ms !== undefined ? job.duration_ms + ' ms' : '-';
                const started = formatTime(job.started_at);

                let statusBadgeHtml = '';
                const code = job.status_code || 0;
                const status = (job.status || 'UNKNOWN').toUpperCase();

                if (code === 200 || status === 'COMPLETED') {
                    statusBadgeHtml = '<span class="status-badge status-200">✅ 200 OK</span>';
                } else if (code === 401 || (status === 'FAILED' && ((job.error_type || '').includes('Credentials') || (job.error_type || '').includes('Auth') || (job.error || '').includes('Invalid username')))) {
                    statusBadgeHtml = '<span class="status-badge status-401">🛑 401 UNAUTH</span>';
                } else if (code === 403 || (status === 'FAILED' && (job.error_type || '').includes('Denied'))) {
                    statusBadgeHtml = '<span class="status-badge status-403">🛑 403 FORBIDDEN</span>';
                } else if (code === 429 || (status === 'FAILED' && (job.error_type || '').includes('RateLimit'))) {
                    statusBadgeHtml = '<span class="status-badge status-429">⚠️ 429 LIMIT</span>';
                } else if (status === 'ROLLED_BACK') {
                    statusBadgeHtml = '<span class="status-badge status-rollback">↩️ ROLLED_BACK</span>';
                } else if (status === 'RUNNING') {
                    statusBadgeHtml = '<span class="status-badge status-running">⏳ RUNNING</span>';
                } else if (status === 'AWAITING_SIGNAL' || status === 'AWAITING_APPROVAL') {
                    // Paused, not failed: waiting for an event or a decision.
                    statusBadgeHtml = '<span class="status-badge status-running">⏸ ' + (status === 'AWAITING_SIGNAL' ? 'WAITING' : 'REVIEW') + '</span>';
                } else {
                    statusBadgeHtml = '<span class="status-badge status-500">🛑 ' + (code > 0 ? code : 'FAILED') + '</span>';
                }

                let methodClass = 'method-post';
                if (method === 'GET') methodClass = 'method-get';
                else if (method === 'PUT' || method === 'PATCH') methodClass = 'method-put';
                else if (method === 'DELETE') methodClass = 'method-delete';

                html += '<tr>' +
                    '<td>' + statusBadgeHtml + '</td>' +
                    '<td><span class="job-id-code" data-inspect-job="' + jId + '" data-wf="' + escapeHtml(rawWf) + '" data-route="' + escapeHtml(rawRoute) + '" data-method="' + method + '" title="Click to inspect in visualizer graph">' + jId + '</span></td>' +
                    '<td><span style="font-weight: 600; color: #cbd5e1;">' + wf + '</span></td>' +
                    '<td><div style="display: flex; align-items: center; gap: 6px;"><span class="method-badge ' + methodClass + '">' + method + '</span><span style="font-family: \'Fira Code\', monospace; color: #94a3b8; font-size: 11px;">' + route + '</span></div></td>' +
                    '<td><span style="font-family: \'Fira Code\', monospace; color: #a5b4fc; font-size: 11px;">🌐 ' + ip + '</span></td>' +
                    '<td><span style="font-family: \'Fira Code\', monospace; color: #fde68a; font-weight: 500;">⚡ ' + duration + '</span></td>' +
                    '<td><span style="color: #64748b; font-size: 11px;">' + started + '</span></td>' +
                    '<td style="text-align: right;"><button class="btn-inspect" data-inspect-job="' + jId + '" data-wf="' + escapeHtml(rawWf) + '" data-route="' + escapeHtml(rawRoute) + '" data-method="' + method + '" title="Inspect this execution in graph"><span>Inspect</span><span>↗</span></button></td>' +
                    '</tr>';
            });

            tbody.innerHTML = html;
            if (!tbody.dataset.inspectBound) {
                // One delegated handler; values come from data-* attributes
                // (decoded as text), never from generated JavaScript.
                tbody.dataset.inspectBound = '1';
                tbody.addEventListener('click', (e) => {
                    const el = e.target.closest('[data-inspect-job]');
                    if (el) selectJobFromModal(el.dataset.inspectJob, el.dataset.wf, el.dataset.route, el.dataset.method);
                });
            }
        }

        function formatTime(isoStr) {
            if (!isoStr) return '-';
            try {
                const date = new Date(isoStr);
                return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
            } catch (e) {
                return isoStr;
            }
        }

        async function selectJobFromModal(jobId, workflow, route, method) {
            closeJobsModal();
            const jobIdInput = document.getElementById('jobIdInput');
            jobIdInput.value = jobId;

            if (workflow && workflow !== '-') {
                const domainSelect = document.getElementById('domainSelect');
                if (domainSelect && domainSelect.value !== workflow) {
                    domainSelect.value = workflow;
                    await onDomainSelectChanged();
                }
            }

            if (route && route !== '-') {
                const routeSelect = document.getElementById('routeSelect');
                if (routeSelect) {
                    for (let i = 0; i < routeSelect.options.length; i++) {
                        if (routeSelect.options[i].value === route) {
                            routeSelect.selectedIndex = i;
                            break;
                        }
                    }
                }
            }

            await lookupJob();
        }

        // Initialize on load
        async function initApp() {
            await refreshDomains();
            loadBlueprintGraph();
        }
        if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', initApp);
        } else {
            initApp();
        }
    </script>
</body>
</html>`

	res := strings.ReplaceAll(tpl, "{{DEFAULT_DOMAIN}}", defaultDomain)
	res = strings.ReplaceAll(res, "{{DEFAULT_PREFIX}}", routePrefix)
	return res
}
