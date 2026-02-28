const options = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
        title: {
            display: true,
            text: (ctx) => 'Lines of code – by repositoriry'
        },
        tooltip: {
            mode: 'point'
        },
        legend: {
            display: true,
            position:"right",
            labels: {
                font: {
                    size: 11,
                }
            },
            reverse: true,
            maxWidth: 260,
        }
    },
    interaction: {
        mode: 'nearest',
        axis: 'x',
        intersect: false
    },
    scales: {
        x: {
            title: {
                display: true,
                text: 'Month'
            }
        },
        y: {
            stacked: true,
            title: {
                display: true,
                text: 'Lines'
            }
        }
    },
    elements: {
        point: {
            radius: 1,
            hoverRadius: 2,
        }
    }
};

const by_repo_div = document.getElementById('by_repo_div');
const by_lang_div = document.getElementById('by_lang_div');

new Chart(by_repo_div, { type: 'line', data: by_repo_data, options: options });
new Chart(by_lang_div, { type: 'line', data: by_lang_data, options: options });

// initial state
by_repo_div.style.display = 'block';
by_lang_div.style.display = 'none';

document.getElementById('by_repo').addEventListener('click', function(event) {
    event.preventDefault(); // prevents the page from jumping to the top
    by_repo_div.style.display = 'block';
    by_lang_div.style.display = 'none';
});

document.getElementById('by_lang').addEventListener('click', function(event) {
    event.preventDefault(); // prevents the page from jumping to the top
    by_repo_div.style.display = 'none';
    by_lang_div.style.display = 'block';
});