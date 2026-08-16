/**
 * MYANMAR FOOTBALL ODDS SETTLEMENT ENGINE
 */

function parseMyanmarOdd(oddStr) {
  if (!oddStr) return { hdc: 0, sign: '-', val: 0 };
  const str = oddStr.toString().trim();

  const match = str.match(/^([0-9.]+)?([+-])([0-9]+)$/);
  if (match) {
    return {
      hdc: match[1] ? parseFloat(match[1]) : 0,
      sign: match[2],
      val: parseInt(match[3], 10)
    };
  }

  const plainHdc = parseFloat(str);
  return { hdc: isNaN(plainHdc) ? 0 : plainHdc, sign: '-', val: 0 };
}

function getOppositeOdd(oddStr) {
  const parsed = parseMyanmarOdd(oddStr);
  const oppSign = parsed.sign === '-' ? '+' : '-';
  return `${parsed.hdc > 0 ? parsed.hdc : ''}${oppSign}${parsed.val}`;
}

function settleMyanmarBet(bet, match, homeScore, awayScore) {
  const amount = parseFloat(bet.amount);
  const choice = bet.choice.toString().trim().toUpperCase();
  const betType = bet.bet_type;
  const betOddStr = bet.odd;
  const parsedOdd = parseMyanmarOdd(betOddStr);
  const rate = parsedOdd.val / 100;

  let isFullWin = false;
  let isFullLost = false;
  let isLevel = false;

  // 1. OVER / UNDER
  if (betType === 'OVER_UNDER') {
    const totalGoals = homeScore + awayScore;
    const hdc = parsedOdd.hdc;
    const isBetOver = choice.includes('OVER');

    if (totalGoals > hdc) {
      if (isBetOver) isFullWin = true;
      else isFullLost = true;
    } else if (totalGoals < hdc) {
      if (isBetOver) isFullLost = true;
      else isFullWin = true;
    } else {
      isLevel = true;
    }
  }

  // 2. BODY
  if (betType === 'BODY') {
    const hdc = parsedOdd.hdc;
    const isHomeUpper = (match.upper_team === 'HOME');
    const isBetHome = choice.includes(match.home_team.toUpperCase()) || choice.includes('HOME');
    const isBetUpper = isHomeUpper ? isBetHome : !isBetHome;

    let goalDiff = 0;
    if (isHomeUpper) {
      goalDiff = homeScore - awayScore;
    } else {
      goalDiff = awayScore - homeScore;
    }

    if (goalDiff > hdc) {
      if (isBetUpper) isFullWin = true;
      else isFullLost = true;
    } else if (goalDiff < hdc) {
      if (isBetUpper) isFullLost = true;
      else isFullWin = true;
    } else {
      isLevel = true;
    }
  }

  // 3. PAYOUT & VALID AMOUNT CALCULATION

  // အပြည့်နိုင်
  if (isFullWin) {
    return { status: 'WON', payout: amount * 2, valid_amount: amount };
  }

  // အပြည့်ရှုံး
  if (isFullLost) {
    return { status: 'LOST', payout: 0, valid_amount: amount };
  }

  // ကြေးကျချိန် (ပြားစွန်းအလိုက် စား/ပေး တွက်ချက်ခြင်း)
  if (isLevel) {
    if (parsedOdd.val === 0) {
      return { status: 'DRAW', payout: amount, valid_amount: 0 };
    }

    // ပြားစွန်းအလိုက် အမှန်တကယ် လောင်းကြေးဖြစ်သော ပမာဏ
    const effectiveStake = amount * rate;

    if (parsedOdd.sign === '-') {
      // အနုတ်ကြေး: ရှုံးသည် (ကျန်ငွေ ပြန်အမ်း)
      const payout = amount - effectiveStake;
      return { status: 'LOST', payout: payout, valid_amount: effectiveStake };
    } else {
      // အပေါင်းကြေး: နိုင်သည် (မူရင်းလောင်းကြေး + ပြားစွန်းအမြတ်)
      const payout = amount + effectiveStake;
      return { status: 'WON', payout: payout, valid_amount: effectiveStake };
    }
  }

  return { status: 'DRAW', payout: amount, valid_amount: 0 };
}

module.exports = {
  parseMyanmarOdd,
  getOppositeOdd,
  settleMyanmarBet
};