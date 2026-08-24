async function request(path, options = {}) {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...options,
    headers: {
      ...(options.body instanceof FormData ? {} : {'Content-Type': 'application/json'}),
      ...(options.headers || {}),
    },
  });
  if (response.status === 401) {
    location.href = '/login';
    throw new Error('登录已失效');
  }
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.detail || data.message || `请求失败 (${response.status})`);
  return data;
}

export const api = {
  get: (path) => request(path),
  post: (path, body = {}) => request(path, {method: 'POST', body: JSON.stringify(body)}),
  delete: (path, body = {}) => request(path, {method: 'DELETE', body: JSON.stringify(body)}),
  uploadTorrent: (file, title = '') => request('/api/ui/torrent', {
    method: 'POST',
    body: file,
    headers: {
      'Content-Type': 'application/x-bittorrent',
      'X-Filename': encodeURIComponent(file.name),
      'X-Title': encodeURIComponent(title),
    },
  }),
};
