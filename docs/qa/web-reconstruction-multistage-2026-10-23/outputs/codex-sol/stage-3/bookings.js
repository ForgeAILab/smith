const rows = document.getElementById('booking-rows');
const form = document.getElementById('booking-form');
const bookingDialog = document.getElementById('booking-dialog');
const search = document.getElementById('booking-search');
const spaceFilter = document.getElementById('space-filter');
let statusFilter = 'all';
let currentPage = 1;
let addedCount = 0;
function applyFilters() {
  const query = search.value.trim().toLocaleLowerCase();
  const filtered = Array.from(rows.rows).filter(row =>
    (statusFilter === 'all' || row.dataset.status === statusFilter) &&
    (spaceFilter.value === 'all' || row.dataset.space === spaceFilter.value) &&
    (row.dataset.member.toLocaleLowerCase().includes(query) || row.dataset.space.toLocaleLowerCase().includes(query)));
  const visible = filtered.slice((currentPage - 1) * 6, currentPage * 6);
  Array.from(rows.rows).forEach(row => { row.hidden = !visible.includes(row); });
  const empty = document.getElementById('empty-message');
  empty.hidden = visible.length > 0;
  empty.textContent = currentPage > 1 && !visible.length ? 'No additional bookings are available locally.' : 'No bookings match your filters.';
  const isFiltered = query || statusFilter !== 'all' || spaceFilter.value !== 'all';
  const total = isFiltered ? filtered.length : 48 + addedCount;
  document.getElementById('booking-summary').textContent = visible.length ? `Showing ${(currentPage - 1) * 6 + 1}–${(currentPage - 1) * 6 + visible.length} of ${total} bookings` : `Showing 0 of ${total} bookings`;
  document.querySelectorAll('[data-number]').forEach(button => {
    if (Number(button.dataset.number) === currentPage) button.setAttribute('aria-current', 'page');
    else button.removeAttribute('aria-current');
  });
}
document.querySelectorAll('[data-status]').forEach(button => {
  if (button.tagName !== 'BUTTON') return;
  button.addEventListener('click', () => {
    statusFilter = button.dataset.status;
    currentPage = 1;
    document.querySelectorAll('.booking-tabs button').forEach(item => item.setAttribute('aria-pressed', String(item === button)));
    applyFilters();
  });
});
search.addEventListener('input', () => { currentPage = 1; applyFilters(); });
spaceFilter.addEventListener('change', () => { currentPage = 1; applyFilters(); });
document.querySelectorAll('[data-number]').forEach(button => button.addEventListener('click', () => { currentPage = Number(button.dataset.number); applyFilters(); }));
document.querySelectorAll('[data-step]').forEach(button => button.addEventListener('click', () => { currentPage = Math.max(1, Math.min(5, currentPage + Number(button.dataset.step))); applyFilters(); }));
document.getElementById('new-booking').addEventListener('click', () => bookingDialog.showModal());
document.getElementById('cancel-booking').addEventListener('click', () => bookingDialog.close());
bookingDialog.querySelector('.close-dialog').addEventListener('click', () => bookingDialog.close());
form.elements.member.addEventListener('input', () => form.elements.member.setCustomValidity(''));
form.addEventListener('submit', event => {
  event.preventDefault();
  if (!form.reportValidity()) return;
  const values = new FormData(form);
  const name = String(values.get('member')).trim();
  if (!name) { form.elements.member.setCustomValidity('Enter a member name.'); form.elements.member.reportValidity(); return; }
  const space = String(values.get('space'));
  const status = String(values.get('status'));
  const date = new Date(String(values.get('date')));
  const dateText = date.toLocaleDateString('en-US', {month:'short', day:'numeric'}) + ', ' + date.toLocaleTimeString('en-GB', {hour:'2-digit', minute:'2-digit'});
  const row = rows.rows[0].cloneNode(true);
  row.hidden = false;
  row.dataset.member = name; row.dataset.space = space; row.dataset.status = status;
  const initials = name.split(/\s+/).slice(0, 2).map(word => word[0]).join('').toUpperCase();
  row.querySelector('.avatar').textContent = initials;
  row.querySelector('.member-cell > span:last-child').textContent = name;
  row.cells[1].textContent = space;
  row.cells[2].textContent = dateText;
  const badge = row.querySelector('.status'); badge.textContent = status; badge.classList.toggle('pending', status === 'Pending');
  row.querySelector('.row-action').setAttribute('aria-label', 'Booking details for ' + name);
  rows.append(row);
  addedCount += 1;
  document.getElementById('total-count').textContent = 48 + addedCount;
  if (status === 'Pending') document.getElementById('pending-count').textContent = Number(document.getElementById('pending-count').textContent) + 1;
  if (!Array.from(spaceFilter.options).some(option => option.value === space)) spaceFilter.add(new Option(space, space));
  statusFilter = 'all'; search.value = ''; spaceFilter.value = 'all'; currentPage = Math.ceil(rows.rows.length / 6);
  document.querySelectorAll('.booking-tabs button').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.status === 'all')));
  applyFilters(); bookingDialog.close(); form.reset();
  document.getElementById('update-status').textContent = 'Booking added for ' + name;
});
function showDetails(row) {
  const details = document.getElementById('detail-content'); details.replaceChildren();
  [['Member', row.dataset.member], ['Space', row.dataset.space], ['Date & time', row.cells[2].textContent], ['Status', row.dataset.status]].forEach(([label,value]) => {
    const term = document.createElement('dt'), text = document.createElement('dd'); term.textContent = label; text.textContent = value; details.append(term, text);
  });
  document.getElementById('detail-dialog').showModal();
}
rows.addEventListener('click', event => { const button = event.target.closest('.row-action'); if (button) showDetails(button.closest('tr')); });
document.getElementById('next-booking').addEventListener('click', () => showDetails(rows.rows[0]));
document.querySelectorAll('[data-close]').forEach(button => button.addEventListener('click', () => document.getElementById(button.dataset.close).close()));
document.querySelectorAll('button.nav-item').forEach(button => button.addEventListener('click', () => {
  document.querySelectorAll('.nav-item').forEach(item => { item.classList.toggle('active', item === button); item.removeAttribute('aria-current'); });
  button.setAttribute('aria-current','page'); document.getElementById('current-page').textContent = button.dataset.page;
}));
document.getElementById('overview-date').addEventListener('change', event => {
  if (!event.target.value) return;
  document.getElementById('selected-date').textContent = new Date(event.target.value + 'T12:00:00').toLocaleDateString('en-US',{month:'short',day:'numeric',year:'numeric'});
  document.getElementById('date-popover').hidePopover();
});
let weekStart = new Date(2026, 9, 19, 12);
let selectedDate = new Date(2026, 9, 23, 12);
function drawWeek() {
  document.getElementById('week-month').textContent = weekStart.toLocaleDateString('en-US', {month:'long', year:'numeric'});
  document.querySelectorAll('.week-day').forEach((button,index) => {
    const date = new Date(weekStart); date.setDate(date.getDate() + index);
    const selected = date.toDateString() === selectedDate.toDateString();
    button.querySelector('strong').textContent = date.getDate(); button.classList.toggle('selected',selected); button.setAttribute('aria-pressed',String(selected));
    button.setAttribute('aria-label', date.toLocaleDateString('en-US', {weekday:'long',month:'long',day:'numeric',year:'numeric'}));
    button.onclick = () => { selectedDate = date; drawWeek(); };
  });
}
document.getElementById('previous-week').addEventListener('click', () => { weekStart.setDate(weekStart.getDate() - 7); drawWeek(); });
document.getElementById('next-week').addEventListener('click', () => { weekStart.setDate(weekStart.getDate() + 7); drawWeek(); });
let calendarMonth = new Date(2026, 9, 1, 12);
function drawMonth() {
  document.getElementById('calendar-title').textContent = calendarMonth.toLocaleDateString('en-US',{month:'long',year:'numeric'});
  const grid = document.getElementById('month-calendar'); grid.replaceChildren();
  ['Mon','Tue','Wed','Thu','Fri','Sat','Sun'].forEach(day => { const label = document.createElement('span'); label.textContent = day; grid.append(label); });
  const offset = (calendarMonth.getDay() + 6) % 7;
  for (let index=0; index<offset; index++) grid.append(document.createElement('span'));
  const last = new Date(calendarMonth.getFullYear(),calendarMonth.getMonth()+1,0).getDate();
  for (let day=1; day<=last; day++) {
    const date = new Date(calendarMonth.getFullYear(),calendarMonth.getMonth(),day,12), button = document.createElement('button');
    button.textContent = day; button.classList.toggle('selected',date.toDateString() === selectedDate.toDateString()); button.setAttribute('aria-label',date.toLocaleDateString('en-US',{month:'long',day:'numeric',year:'numeric'}));
    button.addEventListener('click', () => { selectedDate = date; weekStart = new Date(date); weekStart.setDate(date.getDate() - (date.getDay()+6)%7); drawWeek(); document.getElementById('calendar-dialog').close(); }); grid.append(button);
  }
}
document.getElementById('view-calendar').addEventListener('click', () => { calendarMonth = new Date(selectedDate.getFullYear(),selectedDate.getMonth(),1,12); drawMonth(); document.getElementById('calendar-dialog').showModal(); });
document.getElementById('previous-month').addEventListener('click', () => { calendarMonth.setMonth(calendarMonth.getMonth()-1); drawMonth(); });
document.getElementById('next-month').addEventListener('click', () => { calendarMonth.setMonth(calendarMonth.getMonth()+1); drawMonth(); });
drawWeek();
