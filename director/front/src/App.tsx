import React, {useState, useEffect} from 'react';
import logo from './logo.svg';
import './App.css';
import Version from "./Version";
import {
    AppBar,
    CssBaseline,
    IconButton,
    Typography,
    Button,
    Toolbar,
    Grid,
    Paper,
    Container,
    Box
} from "@material-ui/core";
import {ThemeProvider} from '@material-ui/core/styles';
import theme, {useStyle} from "./theme";
import ClapIcon from "./Clap";
import {green} from "@material-ui/core/colors";
import { useTheme } from '@material-ui/core/styles';

const Main = () => {
    const classes = useStyle();
    return <Box className={classes.main}>
        <Grid  container spacing={2}>
            <Grid item xs={3}/>
            <Grid item xs={3}/>
            <Grid item xs={3}/>
            <Grid item xs={3}>
                <Paper className={classes.paper}>
                    <Version/>
                </Paper>
            </Grid>

        </Grid>
    </Box>
};

const TopBar = () => {
    const classes = useStyle();
    const theme = useTheme();
    return <AppBar position="static">
        <Toolbar>
            <IconButton color="secondary" edge="start" aria-label="menu">
                <ClapIcon style={{ color: theme.palette.primary.contrastText  }}/>
            </IconButton>
            <Typography  className={classes.headerTitle}>
                Director
            </Typography>

        </Toolbar>
    </AppBar>
}

const App: React.FC = () => {

    return (
        <ThemeProvider theme={theme}>
            <CssBaseline/>
            <TopBar/>
            <Main/>
        </ThemeProvider>
    );
};

export default App;
