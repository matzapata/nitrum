'use strict';

const express = require('express');
const http = require('http');

const PORT = parseInt(process.env.PORT || '8008', 10);
const app = express();

function httpGet(url) {
    return new Promise((resolve, reject) => {
        http.get(url, (res) => {
            let data = '';
            res.on('data', (chunk) => { data += chunk; });
            res.on('end', () => resolve(data));
        }).on('error', reject);
    });
}

app.get('/health', (req, res) => {
    res.send('OK');
});

app.get('/egress', async (req, res) => {
    try {
        const body = await httpGet('http://httpbin.org/ip');
        res.type('json').send(body);
    } catch (err) {
        console.error(`[server] egress error: ${err.message}`);
        res.status(502).json({ error: err.message });
    }
});

app.listen(PORT, '0.0.0.0', () => {
    console.log(`[server] listening on port ${PORT}`);
});
