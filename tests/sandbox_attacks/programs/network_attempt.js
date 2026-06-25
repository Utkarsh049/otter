const net = require('net');
const client = net.connect({host: '8.8.8.8', port: 53}, () => {
    console.log('CONNECTED');
    client.end();
});
client.on('error', () => {
    console.log('FAILED');
});
