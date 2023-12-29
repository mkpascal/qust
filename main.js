// main.js

// Modules to control application life and create native browser window
const { app, BrowserWindow, ipcMain, globalShortcut } = require('electron');
const path = require('node:path')
const Store = require('electron-store');
const store = new Store();

const createWindow = () => {
  // Create the browser window.
  const mainWindow = new BrowserWindow({
    width: 980,
    height: 322,
    title: "Qust",
    resizable: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      nodeIntegration: true,
      contextIsolation: false,
      webSecurity: false
    },
    autoHideMenuBar: true,
  })

  // and load the index.html of the app.
  mainWindow.loadFile('index.html')

  // Open the DevTools.
  //mainWindow.webContents.openDevTools()

  globalShortcut.register('Ctrl+Alt+Insert', () => {
    mainWindow.webContents.send('play-action')
  })

  globalShortcut.register('Ctrl+Alt+H', () => {
    mainWindow.webContents.send('stop-action')
  })

}

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.whenReady().then(createWindow);

// IPC handlers for storing and retrieving data
ipcMain.handle('set-data', (event, key, value) => {
    store.set(key, value);
});

ipcMain.handle('delete-data', (event, key) => {
  store.delete(key);
});

ipcMain.handle('get-data', (event, key) => {
    return store.get(key);
});

app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') {
        app.quit();
    }
});

app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
        createWindow();
    }
});

// Quit when all windows are closed, except on macOS. There, it's common
// for applications and their menu bar to stay active until the user quits
// explicitly with Cmd + Q.
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit()
})

app.on('will-quit', () => {
  globalShortcut.unregisterAll();
});

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and require them here.