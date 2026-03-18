const autocolors = window['chartjs-plugin-autocolors'];

const options = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
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
            maxWidth: 1000,
        },
        autocolors: {
            customize(context) {
                const colors = context.colors;
                return {
                    background: Chart.helpers.color(colors.background).lighten(0.5).alpha(1).rgbString(),
                    border: colors.border
                };
            }
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
                text: 'Month',
                font: { weight: 'bold' }
            }
        },
        y: {
            stacked: true,
            min: 0,
            title: {
                display: true,
                text: 'Lines',
                font: { weight: 'bold' }
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

Chart.register(autocolors);

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