var HttpClient = require('request');
var qs = require('querystring');

var domain = process.argv[2] || 'expl.lichess.org';

var uri = 'http://' + domain + '/lichess';

var speeds = ['bullet', 'blitz', 'classical'];
var ratings = [1600, 1800, 2000, 2200, 2500];

var results = {};
speeds.forEach(function(s) {
  results[s] = {};
});

function request(speed, rating) {
  HttpClient.get({
    url: uri,
    qs: {
      fen: 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
      moves: 12,
      variant: 'standard',
      'speeds[]': speed,
      'ratings[]': rating
    },
    json: true
  }, function(err, res, body) {
    if (err) console.log(err);
    var nb = 0;
    body.moves.forEach(function(m) {
      nb += m.black += m.white += m.draws;
    });
    results[speed][rating] = nb;
    console.log(results);
  });
};

speeds.forEach(function(s) {
  ratings.forEach(function(r) {
    request(s, r);
  });
});
