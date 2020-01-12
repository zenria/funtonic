import {red} from '@material-ui/core/colors';
import {createMuiTheme} from '@material-ui/core/styles';
import makeStyles from "@material-ui/core/styles/makeStyles";

// A custom theme for this app
const theme = createMuiTheme({
    palette: {
        primary: { main: '#000' },
        secondary: { main: '#455A64' },
        error: {
            main: red.A400,
        },
        background: {
            default: "#EEE",

        }
    },
});

export const useStyle = makeStyles(theme => ({
    headerTitle: {
        fontSize: "16px"
    },
    main: {
        padding: theme.spacing(2),
    },
    paper: {
        padding: theme.spacing(2),
        color: theme.palette.text.secondary,
    },
}));

export default theme;
