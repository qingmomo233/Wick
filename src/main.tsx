import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './styles.css';

// 禁用 webview 默认右键菜单（刷新/另存为/检查等浏览器项）
window.addEventListener('contextmenu', (e) => e.preventDefault());

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
