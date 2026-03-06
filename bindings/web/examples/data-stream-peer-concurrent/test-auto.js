import puppeteer from 'puppeteer';

const CLIENT_URL = process.env.CLIENT_URL || 'http://127.0.0.1:4175';
const SERVER_URL = process.env.SERVER_URL || 'http://127.0.0.1:4176';
const CLIENT_COUNT = Number(process.env.CLIENT_COUNT || 2);
const MESSAGE_COUNT = Number(process.env.MESSAGE_COUNT || 3);
const DEFAULT_MAC_CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function getExecutablePath() {
    if (process.env.PUPPETEER_EXECUTABLE_PATH) {
        return process.env.PUPPETEER_EXECUTABLE_PATH;
    }

    if (process.platform === 'darwin') {
        return DEFAULT_MAC_CHROME;
    }

    return undefined;
}

async function waitForText(page, selector, expected, timeout = 45000) {
    await page.waitForFunction(
        ({ selector, expected }) => {
            const el = document.querySelector(selector);
            return !!el && el.textContent && el.textContent.includes(expected);
        },
        { timeout },
        { selector, expected }
    );
}

async function main() {
    const browser = await puppeteer.launch({
        headless: 'new',
        ignoreHTTPSErrors: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--ignore-certificate-errors'],
        executablePath: getExecutablePath(),
    });

    const pages = [];
    let serverPage = null;
    const clientPages = [];

    try {
        serverPage = await browser.newPage();
        pages.push(serverPage);
        serverPage.on('console', (msg) => console.log('[server]', msg.text()));
        await serverPage.goto(SERVER_URL, { waitUntil: 'networkidle2' });
        await waitForText(serverPage, '#status', '运行中');

        for (let i = 1; i <= CLIENT_COUNT; i += 1) {
            const page = await browser.newPage();
            pages.push(page);
            clientPages.push(page);
            page.on('console', (msg) => console.log(`[client-${i}]`, msg.text()));
            const url = `${CLIENT_URL}?autoStart=1&clientId=client-${i}&messageCount=${MESSAGE_COUNT}`;
            await page.goto(url, { waitUntil: 'networkidle2' });
            await waitForText(page, '#status', '已连接');
        }

        await new Promise((resolve) => setTimeout(resolve, 12000));

        for (let i = 1; i <= CLIENT_COUNT; i += 1) {
            const page = clientPages[i - 1];
            const logText = await page.$eval('#log', (el) => el.textContent || '');
            if (!logText.includes(`client received ${MESSAGE_COUNT}/${MESSAGE_COUNT}`)) {
                throw new Error(`client-${i} missing receive completion log`);
            }
            if (!logText.includes(`client sending ${MESSAGE_COUNT}/${MESSAGE_COUNT}`)) {
                throw new Error(`client-${i} missing send completion log`);
            }
        }

        console.log('✅ data-stream peer concurrent test passed');
    } catch (error) {
        if (serverPage) {
            try {
                console.error('[debug] server status:', await serverPage.$eval('#status', (el) => el.textContent));
                console.error('[debug] server log:\n' + await serverPage.$eval('#log', (el) => el.textContent));
            } catch { }
        }

        for (let i = 0; i < clientPages.length; i += 1) {
            const page = clientPages[i];
            try {
                console.error(`[debug] client-${i + 1} status:`, await page.$eval('#status', (el) => el.textContent));
                console.error(`[debug] client-${i + 1} log:\n${await page.$eval('#log', (el) => el.textContent)}`);
            } catch { }
        }

        throw error;
    } finally {
        await Promise.all(pages.map((page) => page.close().catch(() => { })));
        await browser.close();
    }
}

main().catch((error) => {
    console.error('❌ data-stream peer concurrent test failed:', error);
    process.exitCode = 1;
});
