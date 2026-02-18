function drawCharts(by_repo_data, by_lang_data) {
    drawByRepoChart(by_repo_data);
    drawByLangChart(by_lang_data);
}

function drawByRepoChart(raw_data) {
    let data = google.visualization.arrayToDataTable(raw_data);
    let options = {
        isStacked: true,
        title: 'Lines of code – by repository',
        hAxis: {title: 'Month', titleTextStyle: {color: '#333'}},
        vAxis: {minValue: 0},
        lineWidth: 1,
    };
    let chart = new google.visualization.AreaChart(document.getElementById('by_repo_div'));
    chart.draw(data, options);
}

function drawByLangChart(raw_data) {
    let data = google.visualization.arrayToDataTable(raw_data);
    let options = {
        isStacked: true,
        title: 'Line of code – by language',
        hAxis: {title: 'Month', titleTextStyle: {color: '#333'}},
        vAxis: {minValue: 0},
        lineWidth: 1,
    };
    let chart = new google.visualization.AreaChart(document.getElementById('by_lang_div'));
    chart.draw(data, options);
}

google.charts.load('current', {'packages':['corechart']});
google.charts.setOnLoadCallback(() => drawCharts(by_repo_data, by_lang_data));
