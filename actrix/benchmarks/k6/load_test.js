// Actrix K6 负载测试脚本
import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate } from 'k6/metrics';

// 自定义指标
const errorRate = new Rate('errors');

export const options = {
  stages: [
    { duration: '30s', target: 50 },   // 爬坡到 50 VU
    { duration: '1m', target: 100 },   // 爬坡到 100 VU
    { duration: '2m', target: 100 },   // 稳定在 100 VU
    { duration: '30s', target: 0 },    // 降回 0
  ],
  thresholds: {
    http_req_duration: ['p(95)<200', 'p(99)<500'], // 95% < 200ms, 99% < 500ms
    http_req_failed: ['rate<0.01'],                 // 错误率 < 1%
    errors: ['rate<0.1'],                           // 自定义错误率 < 10%
  },
};

const BASE_URL = 'https://localhost:8443';

export default function () {
  // 1. Health Check
  let healthRes = http.get(`${BASE_URL}/ks/health`, {
    tags: { name: 'HealthCheck' },
  });

  check(healthRes, {
    'health status is 200': (r) => r.status === 200,
    'health response time < 50ms': (r) => r.timings.duration < 50,
  }) || errorRate.add(1);

  sleep(0.1);

  // 2. Metrics Endpoint
  let metricsRes = http.get(`${BASE_URL}/metrics`, {
    tags: { name: 'Metrics' },
  });

  check(metricsRes, {
    'metrics status is 200': (r) => r.status === 200,
    'metrics contains actrix_': (r) => r.body.includes('actrix_'),
  }) || errorRate.add(1);

  sleep(1);
}

export function handleSummary(data) {
  return {
    'benchmarks/results/k6_summary.html': htmlReport(data),
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
  };
}

function htmlReport(data) {
  return `
<!DOCTYPE html>
<html>
<head>
  <title>Actrix K6 Load Test Report</title>
  <style>
    body { font-family: Arial, sans-serif; margin: 20px; }
    h1 { color: #333; }
    table { border-collapse: collapse; width: 100%; margin-top: 20px; }
    th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
    th { background-color: #4CAF50; color: white; }
    .pass { color: green; }
    .fail { color: red; }
  </style>
</head>
<body>
  <h1>Actrix Load Test Report</h1>
  <p>Test Date: ${new Date().toISOString()}</p>

  <h2>Summary</h2>
  <table>
    <tr><th>Metric</th><th>Value</th></tr>
    <tr><td>Total Requests</td><td>${data.metrics.http_reqs.values.count}</td></tr>
    <tr><td>Failed Requests</td><td>${data.metrics.http_req_failed.values.rate * 100}%</td></tr>
    <tr><td>Avg Duration</td><td>${data.metrics.http_req_duration.values.avg.toFixed(2)} ms</td></tr>
    <tr><td>P95 Duration</td><td>${data.metrics.http_req_duration.values['p(95)'].toFixed(2)} ms</td></tr>
    <tr><td>P99 Duration</td><td>${data.metrics.http_req_duration.values['p(99)'].toFixed(2)} ms</td></tr>
  </table>

  <h2>Thresholds</h2>
  <table>
    <tr><th>Threshold</th><th>Status</th></tr>
    ${Object.keys(data.thresholds).map(name => {
      const threshold = data.thresholds[name];
      const passed = Object.values(threshold).every(t => t.ok);
      return `<tr><td>${name}</td><td class="${passed ? 'pass' : 'fail'}">${passed ? '✅ PASS' : '❌ FAIL'}</td></tr>`;
    }).join('')}
  </table>
</body>
</html>
  `;
}

function textSummary(data, options) {
  // 简化的文本汇总
  return `
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  K6 Load Test Summary
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Total Requests: ${data.metrics.http_reqs.values.count}
  Failed: ${(data.metrics.http_req_failed.values.rate * 100).toFixed(2)}%
  Avg Duration: ${data.metrics.http_req_duration.values.avg.toFixed(2)} ms
  P95: ${data.metrics.http_req_duration.values['p(95)'].toFixed(2)} ms
  P99: ${data.metrics.http_req_duration.values['p(99)'].toFixed(2)} ms
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  `;
}
