window.AudioContext = window.AudioContext || window.webkitAudioContext || window.mozAudioContext;

let audio    = document.getElementById('audio')
let ctx      = ""
let audioSrc = ""

var globalPlayerClicked = "";
var intervalId;
var titleP;
var urlP;

function start(){
    if (!ctx){
        ctx = new AudioContext();
        analyser = ctx.createAnalyser();
        audioSrc = ctx.createMediaElementSource(audio)
        audioSrc.connect(analyser)
        analyser.connect(ctx.destination)
    }

    var frequencyData = new Uint8Array(analyser.frequencyBinCount);

    var canvas = document.getElementById('canvas'),
        cwidth = canvas.width,
        cheight = canvas.height + 100,
        meterWidth = 10,
        gap = 2,
        capHeight = 2,
        capStyle = '#fff',
        meterNum = 800 / (10 + 2),
        capYPositionArray = [];
    ctx = canvas.getContext('2d'),
    gradient = ctx.createLinearGradient(0, 0, 0, 300);
    gradient.addColorStop(1, '#44bd32');
    gradient.addColorStop(0.5, '#4cd137');
    gradient.addColorStop(0.25, '#F79F1F');
    gradient.addColorStop(0, '#e84118');

    function renderFrame() {
        var array = new Uint8Array(analyser.frequencyBinCount);
        analyser.getByteFrequencyData(array);
        var step = Math.round(array.length / meterNum);
        ctx.clearRect(0, 0, cwidth, cheight);
        for (var i = 0; i < meterNum; i++) {
            var value = array[i * step];
            if (capYPositionArray.length < Math.round(meterNum)) {
                capYPositionArray.push(value);
            };
            ctx.fillStyle = capStyle;
            if (value < capYPositionArray[i]) {
                ctx.fillRect(i * 12, cheight - (--capYPositionArray[i]), meterWidth, capHeight);
            } else {
                ctx.fillRect(i * 12, cheight - value, meterWidth, capHeight);
                capYPositionArray[i] = value;
            };
            ctx.fillStyle = gradient;
            ctx.fillRect(i * 12, cheight - value + capHeight, meterWidth, cheight);
        }
        requestAnimationFrame(renderFrame);
    }
    renderFrame();
    changeVisibilityOfNowPlaying('visible');

    intervalId = setInterval(function() {
        getMetaDataFromStream(urlP)
            .then(title => changeCardStatusText("<b>Now playing:</b> " + title + " (" + titleP + ")"))
            .catch(error => console.error(error));
    }, 15000);

    audio.play();
}

function stop(){
    changeVisibilityOfNowPlaying('hidden');
    clearInterval(intervalId);
    audio.pause()
}

document.addEventListener('DOMContentLoaded', function() {
    const cardFooters = document.querySelectorAll('.card-footer');
    const cardBodies = document.querySelectorAll('.card-body-player');

    const customPrompt = document.getElementById('customPrompt');
    
    const submitBtn = document.getElementById('submitBtn');
    const cancelBtn = document.getElementById('cancelBtn');
    const emptyBtn = document.getElementById('emptyBtn');

    const streamTitleInput = document.getElementById('streamTitle');
    const streamURLInput = document.getElementById('streamURL');

    cardBodies.forEach(function(cardBody) {
        cardBody.addEventListener('click', function() {
            var parentDiv = this.parentElement;
            var parentClass = parentDiv.className;
    
            if (parentClass.includes("player")) {
                var parts = parentClass.split(" ");
                var CBplayer = parts.find(part => part.includes("player"));
            }

            if (parentClass.includes("text-bg-secondary")) {                
            } else {
                loadData(CBplayer + '-title').then(title => {
                    titleP = title;
                    return loadData(CBplayer + '-url');
                }).then(url => {
                    urlP = url;

                    var pppDivs = document.querySelectorAll('.ppp');

                    pppDivs.forEach(element => {
                        var CurrentBgSuc = Array.from(element.classList).find(cls => cls.includes("text-bg-success"));

                        if(CurrentBgSuc) {
                            element.classList.remove('text-bg-success');
                            element.classList.add('text-bg-warning');
                        }

                    });
    
                    var playerDiv = document.querySelector('.' + CBplayer);
                    if (playerDiv) {
                        playerDiv.classList.remove('text-bg-warning');
                        playerDiv.classList.add('text-bg-success');

                        var audioPlayer = document.getElementById('audio');
                        audioPlayer.src = urlP;

                        changeCardStatusText("<b>Now playing:</b> " + titleP + " - " + urlP);

                        // getMetaDataFromStream(urlP)
                        // .then(title => console.log('Current Title:', title))
                        // .catch(error => console.error(error));

                        audioPlayer.load();
                        
                        start();
                    } 

                }).catch(error => {
                    console.error('Error loading data:', error);
                });
            }
        
        });
    });

    cardFooters.forEach(function(cardFooter) {
        cardFooter.addEventListener('click', function() {
            var parentDiv = this.parentElement;
            var parentClass = parentDiv.className;
    
            globalPlayerClicked = parentClass;
    
            customPrompt.style.display = 'flex';
        });
    });

    submitBtn.addEventListener('click', function() {
        if (globalPlayerClicked.includes("player")) {
            var parts = globalPlayerClicked.split(" ");
            var Cplayer = parts.find(part => part.includes("player"));
        }

        const streamTitle = streamTitleInput.value;
        const streamURL = streamURLInput.value;

        saveData(Cplayer + '-title', streamTitle);
        saveData(Cplayer + '-url', streamURL);

        var titleSpan = document.querySelector('.' + Cplayer + '-title');
        if (titleSpan) {
            titleSpan.innerHTML = streamTitle;
        }

        var playerDiv = document.querySelector('.' + Cplayer);
        if (playerDiv) {
            playerDiv.classList.remove('text-bg-secondary');
            playerDiv.classList.add('text-bg-warning');
        }

        customPrompt.style.display = 'none';
        clearInputs();
    });

    cancelBtn.addEventListener('click', function() {
        customPrompt.style.display = 'none';
        clearInputs();
    });

    emptyBtn.addEventListener('click', function() {
        if (globalPlayerClicked.includes("player")) {
            var parts = globalPlayerClicked.split(" ");
            var Cplayer = parts.find(part => part.includes("player"));
        }

        var titleSpan = document.querySelector('.' + Cplayer + '-title');
        if (titleSpan) {
            titleSpan.innerHTML = "&nbsp;";
        }

        var playerDiv = document.querySelector('.' + Cplayer);
        if (playerDiv) {
            playerDiv.classList.add('text-bg-secondary');
            playerDiv.classList.remove('text-bg-success');
            playerDiv.classList.remove('text-bg-warning');
        }
        
        customPrompt.style.display = 'none';
        deleteData(Cplayer + '-title');
        deleteData(Cplayer + '-url');
        clearInputs();
    });

    function clearInputs() {
        streamTitleInput.value = '';
        streamURLInput.value = '';
    }
});


document.addEventListener('DOMContentLoaded', async () => {
    const variablesList = [];
    const numberOfPlayers = 6;
    
    for (let i = 1; i <= numberOfPlayers; i++) {
        variablesList.push(`player${i}-title`);
        variablesList.push(`player${i}-url`);
    }

    
    
    for (const key of variablesList) {
        try {
            const data = await loadData(key);

            if (typeof data !== 'undefined') {
                var playerAlone = key.split('-')[0];

                var playerDiv = document.querySelector('.' + playerAlone);
                if (playerDiv) {
                    playerDiv.classList.remove('text-bg-secondary');
                    playerDiv.classList.add('text-bg-warning');
                }

                if (key.includes("-title")){
                    var titleSpan = document.querySelector('.' + playerAlone + '-title');
                    if (titleSpan) {
                        titleSpan.innerHTML = data;
                    }
                }
            } else {
            }
        } catch (error) {
            console.error(`Error loading ${key}:`, error);
        }
    }
});

document.addEventListener("DOMContentLoaded", function() {
    var audioElement = document.getElementById("audio");
    var volumeDisplay = document.getElementById("volumeDisplay");
    var volumeUp = document.getElementById("volumeUp");
    var volumeDown = document.getElementById("volumeDown");

    var volume = 0.5;
    audioElement.volume = volume;

    function updateVolume(change) {
        volume += change;
        volume = Math.max(0, Math.min(1, volume));
        audioElement.volume = volume;
        volumeDisplay.innerHTML = `<small>${Math.round(volume * 100)}%</small>`;
    }

    volumeUp.addEventListener("click", function() {
        updateVolume(0.1);
    });

    volumeDown.addEventListener("click", function() {
        updateVolume(-0.1);
    });
});

function changeCardStatusText(newText) {
    var cardStatusTextElement = document.querySelector('.card-status-text');
    if(cardStatusTextElement) {
        cardStatusTextElement.innerHTML = newText;
    } else {
        console.log("Element with class 'card-status-text' not found.");
    }
}

function changeVisibilityOfNowPlaying(visibility) {
    var elements = document.querySelectorAll('.nowplaying');
    elements.forEach(function(element) {
        element.style.visibility = visibility;
    });
}

async function getMetaDataFromStream(streamUrl) {
    try {
        let statusUrl = new URL(streamUrl);
        statusUrl.pathname = '/status-json.xsl';

        const response = await fetch(statusUrl.href);
        const data = await response.json();

        if (streamUrl.endsWith('/')) {
            const firstStreamInfo = data.icestats.source[0];
            return firstStreamInfo && firstStreamInfo.title ? firstStreamInfo.title : null;
        } else {
            const streamPath = new URL(streamUrl).pathname;
            const streamInfo = data.icestats.source.find(source => {
                const sourcePath = new URL(source.listenurl).pathname;
                return sourcePath === streamPath;
            });

            if (streamInfo && streamInfo.title) {
                return streamInfo.title;
            } else {
                console.log('Specific stream not found or no title available');
                return "N/A";
            }
        }
    } catch (error) {
        console.error('Error fetching stream metadata:', error);
        return "N/A";
    }
}