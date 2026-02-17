// Add homepage class to body for conditional styling
function checkHomepage() {
    var path = window.location.pathname;
    var isHome = path === '/' || path === '' || path.match(/\/(index\.html?)?$/);
    if (isHome) {
        document.body.classList.add('homepage');
    } else {
        document.body.classList.remove('homepage');
    }
}

// Run on initial load
checkHomepage();

// Run on DOM ready
document.addEventListener('DOMContentLoaded', checkHomepage);

// Handle Material instant navigation
if (typeof document$ !== 'undefined') {
    document$.subscribe(checkHomepage);
}
